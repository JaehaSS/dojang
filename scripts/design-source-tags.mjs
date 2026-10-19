import { relative, resolve, sep } from "node:path";

const INSPECTOR_DIRECTORY = "src/design/";
const SCREEN_DIRECTORY = "src/design/screens/";

function sourcePath(filename, root) {
  if (!filename || filename.startsWith("\0")) return null;

  const absoluteRoot = resolve(root);
  const absoluteFile = resolve(filename);
  const relativeFile = relative(absoluteRoot, absoluteFile).split(sep).join("/");
  if (!relativeFile.startsWith("src/") || relativeFile.startsWith("../") || relativeFile.includes("/node_modules/")) {
    return null;
  }

  // The Design canvas itself is inspector chrome. Its screen fixtures are still
  // preview content and therefore need source locations.
  if (relativeFile.startsWith(INSPECTOR_DIRECTORY) && !relativeFile.startsWith(SCREEN_DIRECTORY)) {
    return null;
  }

  return relativeFile;
}

function isIntrinsicElement(name) {
  return name?.type === "JSXIdentifier" && /^[a-z]/.test(name.name);
}

function isCustomElement(name) {
  if (name?.type === "JSXIdentifier") return /^[A-Z]/.test(name.name);
  return name?.type === "JSXMemberExpression" && isCustomElement(name.object);
}

function classKind(attributes) {
  if (attributes.some((attribute) => attribute.type === "JSXSpreadAttribute")) return "dynamic";

  const className = attributes.find((attribute) => (
    attribute.type === "JSXAttribute" && attribute.name?.name === "className"
  ));
  if (!className) return "none";
  if (className.value?.type === "StringLiteral") return "static";

  const expression = className.value?.type === "JSXExpressionContainer"
    ? className.value.expression
    : null;
  return expression?.type === "StringLiteral" ? "static" : "dynamic";
}

function metadataAttribute(types, name, value) {
  return types.jsxAttribute(types.jsxIdentifier(name), types.stringLiteral(value));
}

function staticMetaValue(node) {
  if (!node) return { ok: false };
  if (node.type === "StringLiteral" || node.type === "NumericLiteral" || node.type === "BooleanLiteral") return { ok: true, value: node.value };
  if (node.type === "NullLiteral") return { ok: true, value: null };
  if (node.type === "UnaryExpression" && node.operator === "-" && node.argument?.type === "NumericLiteral") return { ok: true, value: -node.argument.value };
  if (node.type === "ArrayExpression") {
    const values = node.elements.map(staticMetaValue);
    return values.every((item) => item.ok) ? { ok: true, value: values.map((item) => item.value) } : { ok: false };
  }
  if (node.type === "ObjectExpression") {
    const entries = [];
    for (const property of node.properties) {
      if (property.type !== "ObjectProperty" || property.computed || property.method) return { ok: false };
      const key = property.key.type === "Identifier" ? property.key.name : property.key.type === "StringLiteral" || property.key.type === "NumericLiteral" ? String(property.key.value) : null;
      const value = staticMetaValue(property.value);
      if (key == null || !value.ok) return { ok: false };
      entries.push([key, value.value]);
    }
    return { ok: true, value: Object.fromEntries(entries) };
  }
  return { ok: false };
}

function cacheMember(types) {
  return types.memberExpression(types.identifier("globalThis"), types.identifier("__dojangDesignMetaCache"));
}

function cachedMetaExpression(types, cache, key, value) {
  const keyNode = types.stringLiteral(key);
  const call = (method) => types.callExpression(types.memberExpression(types.cloneNode(cache), types.identifier(method)), [types.cloneNode(keyNode)]);
  return types.conditionalExpression(
    call("has"),
    call("get"),
    types.sequenceExpression([
      types.callExpression(types.memberExpression(types.cloneNode(cache), types.identifier("set")), [types.cloneNode(keyNode), value]),
      call("get"),
    ]),
  );
}

/**
 * Annotates real DOM elements with their source location for the development
 * Design canvas. It deliberately does not evaluate JSX expressions.
 */
export default function designSourceTags(api, options = {}) {
  const { types } = api;
  const root = options.root ?? process.cwd();

  return {
    name: "dojang-design-source-tags",
    visitor: {
      Program: {
        enter(path, state) {
          state.dojangMetaCache = null;
          state.dojangMetaCached = false;
        },
        exit(path, state) {
          if (!state.dojangMetaCached || !state.dojangMetaCache) return;
          const cache = state.dojangMetaCache;
          path.unshiftContainer("body", types.variableDeclaration("const", [
            types.variableDeclarator(
              cache,
              types.logicalExpression(
                "||",
                cacheMember(types),
                types.assignmentExpression("=", cacheMember(types), types.newExpression(types.identifier("Map"), [])),
              ),
            ),
          ]));
        },
      },
      ExportNamedDeclaration(path, state) {
        const file = sourcePath(state.file?.opts?.filename, root);
        if (!file?.startsWith(SCREEN_DIRECTORY) || path.node.declaration?.type !== "VariableDeclaration") return;
        for (const declarator of path.node.declaration.declarations) {
          if (declarator.id?.type !== "Identifier" || declarator.id.name !== "meta") continue;
          const staticValue = staticMetaValue(declarator.init);
          if (!staticValue.ok) continue;
          const cache = state.dojangMetaCache ?? (state.dojangMetaCache = path.scope.generateUidIdentifier("dojangMetaCache"));
          const fingerprint = `${file}\u0000${JSON.stringify(staticValue.value)}`;
          declarator.init = cachedMetaExpression(types, cache, fingerprint, declarator.init);
          state.dojangMetaCached = true;
        }
      },
      JSXOpeningElement(path, state) {
        const file = sourcePath(state.file?.opts?.filename, root);
        if (!file || !path.node.loc) return;

        const intrinsic = isIntrinsicElement(path.node.name);
        const custom = isCustomElement(path.node.name);
        if (!intrinsic && !custom) return;

        const location = path.node.loc.start;
        const metadata = `${file}:${location.line}:${location.column}`;
        const kind = classKind(path.node.attributes);

        const sourceAttribute = intrinsic ? "data-praxis-src" : "data-dojang-instance-src";
        const kindAttribute = intrinsic ? "data-praxis-class-kind" : "data-dojang-instance-kind";

        // Strip spoofed values first, then append ours after every spread so the
        // DOM or React composite props always expose the metadata calculated from
        // this source file. Composite markers live on React props; React 19's
        // nearest fiber lookup can read them even when a component does not
        // forward unknown props to its DOM root.
        path.node.attributes = path.node.attributes.filter((attribute) => (
          attribute.type !== "JSXAttribute"
          || (attribute.name.name !== sourceAttribute && attribute.name.name !== kindAttribute)
        ));
        path.node.attributes.push(
          metadataAttribute(types, sourceAttribute, metadata),
          metadataAttribute(types, kindAttribute, kind),
        );
      },
    },
  };
}
