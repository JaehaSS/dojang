import { createHash } from "node:crypto";
import { existsSync, readFileSync, realpathSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, extname, normalize, relative, resolve, sep } from "node:path";

const SOURCE_LOCATION = /^(.*):(\d+):(\d+)$/;
const EXTENSIONS = [".tsx", ".ts", ".jsx", ".js"];
const REASON_MESSAGES = {
  invalid_request: "The design edit request is invalid.",
  path_outside_worktree: "The requested source is outside the worktree.",
  source_not_found: "The requested JSX source location was not found.",
  invalid_instance: "The instance source does not reference the selected component.",
  dynamic_classname: "The selected className is dynamic or has a spread attribute.",
  no_classname_prop: "This component does not accept a className prop.",
  stale_file: "The source file changed after it was selected.",
  unsupported_property: "This CSS property cannot be written as a Tailwind class.",
  unsafe_value: "This CSS value cannot be represented safely as a Tailwind arbitrary value.",
};

function fail(reason, message = REASON_MESSAGES[reason]) {
  return { ok: false, reason, message };
}

function digest(content) {
  return createHash("sha256").update(content).digest("hex");
}

function parseLocation(value) {
  if (typeof value !== "string") return null;
  const match = value.match(SOURCE_LOCATION);
  if (!match) return null;
  return { file: match[1], line: Number(match[2]), column: Number(match[3]) };
}

function locationString(file, node) {
  return `${file}:${node.loc.start.line}:${node.loc.start.column}`;
}

function worktreePath(root, file) {
  if (typeof file !== "string" || file.includes("\\")) return null;
  const rootReal = realpathSync(root);
  const candidate = resolve(rootReal, file);
  const normalized = relative(rootReal, candidate);
  if (!normalized || normalized.startsWith("..") || normalize(normalized).startsWith(`..${sep}`)) return null;
  if (!existsSync(candidate)) return null;
  const actual = realpathSync(candidate);
  const actualRelative = relative(rootReal, actual);
  if (actualRelative.startsWith("..") || normalize(actualRelative).startsWith(`..${sep}`)) return null;
  return { absolute: actual, file: actualRelative.split(sep).join("/") };
}

function parserFor(root) {
  const require = createRequire(resolve(root, "package.json"));
  return require("@babel/parser");
}

function parseCode(parser, content, filename) {
  return parser.parse(content, {
    sourceType: "module",
    sourceFilename: filename,
    plugins: ["jsx", "typescript"],
  });
}

function walk(node, visit, parent = null) {
  if (!node || typeof node !== "object") return;
  if (typeof node.type === "string") visit(node, parent);
  for (const [key, value] of Object.entries(node)) {
    if (key === "loc" || key === "extra" || key === "comments") continue;
    if (Array.isArray(value)) {
      for (const child of value) walk(child, visit, node);
    } else if (value && typeof value.type === "string") {
      walk(value, visit, node);
    }
  }
}

function jsxName(node) {
  return node?.type === "JSXIdentifier" ? node.name : null;
}

function isIntrinsic(opening) {
  const name = jsxName(opening.name);
  return Boolean(name && /^[a-z]/.test(name));
}

function findOpening(ast, line, column) {
  let found = null;
  walk(ast, (node) => {
    if (node.type === "JSXOpeningElement" && node.loc?.start.line === line && node.loc.start.column === column) {
      found = node;
    }
  });
  return found;
}

function findOpeningAtStart(ast, start) {
  let found = null;
  walk(ast, (node) => {
    if (node.type === "JSXOpeningElement" && node.start === start) found = node;
  });
  return found;
}

function hasSpread(opening) {
  return opening.attributes.some((attribute) => attribute.type === "JSXSpreadAttribute");
}

function classAttribute(opening) {
  return opening.attributes.find((attribute) => attribute.type === "JSXAttribute" && attribute.name?.name === "className") ?? null;
}

function staticClass(attribute) {
  if (!attribute) return { kind: "none", value: "", literal: null };
  if (attribute.value?.type === "StringLiteral") return { kind: "static", value: attribute.value.value, literal: attribute.value };
  if (attribute.value?.type === "JSXExpressionContainer" && attribute.value.expression?.type === "StringLiteral") {
    return { kind: "static", value: attribute.value.expression.value, literal: attribute.value.expression };
  }
  return { kind: "dynamic", value: null, literal: null };
}

function rawClassValue(content, opening) {
  const state = staticClass(classAttribute(opening));
  if (!state.literal) return "";
  return content.slice(state.literal.start + 1, state.literal.end - 1);
}

function classKind(opening) {
  if (hasSpread(opening)) return "dynamic";
  return staticClass(classAttribute(opening)).kind;
}

function enclosingComponent(ast, opening) {
  let answer = null;
  walk(ast, (node, parent) => {
    if (node.start == null || node.end == null || node.start > opening.start || node.end < opening.end) return;
    if (node.type === "FunctionDeclaration" && node.id?.name) {
      if (!answer || node.end - node.start < answer.node.end - answer.node.start) answer = { node, name: node.id.name, params: node.params };
    }
    if ((node.type === "ArrowFunctionExpression" || node.type === "FunctionExpression")
      && parent?.type === "VariableDeclarator" && parent.id?.type === "Identifier") {
      if (!answer || node.end - node.start < answer.node.end - answer.node.start) answer = { node, name: parent.id.name, params: node.params };
    }
  });
  return answer;
}

function hasNamedClassProperty(typeNode, name) {
  if (!typeNode) return false;
  if (typeNode.type === "TSTypeLiteral") {
    return typeNode.members.some((member) => member.type === "TSPropertySignature" && member.key?.name === "className");
  }
  if (typeNode.type === "TSInterfaceBody") return hasNamedClassProperty({ type: "TSTypeLiteral", members: typeNode.body }, name);
  if (typeNode.type === "TSTypeReference" && typeNode.typeName?.type === "Identifier") {
    const target = typeNode.typeName.name;
    let match = false;
    walk(name, (node) => {
      if ((node.type === "TSInterfaceDeclaration" || node.type === "TSTypeAliasDeclaration") && node.id?.name === target) {
        const body = node.body?.typeAnnotation ?? node.body;
        if (hasNamedClassProperty(body, name)) match = true;
      }
    });
    return match;
  }
  return false;
}

function componentAcceptsClassName(component, ast) {
  const param = component?.params?.[0];
  if (!param) return false;
  if (param.type === "ObjectPattern") {
    return param.properties.some((property) => property.type === "ObjectProperty" && property.key?.name === "className");
  }
  if (param.type !== "Identifier") return false;
  if (hasNamedClassProperty(param.typeAnnotation?.typeAnnotation, ast)) return true;
  let readsClassName = false;
  walk(component.node.body, (node) => {
    if (node.type === "MemberExpression" && node.object?.type === "Identifier" && node.object.name === param.name
      && !node.computed && node.property?.name === "className") readsClassName = true;
  });
  return readsClassName;
}

function classNameReachesOpening(component, opening) {
  const param = component?.params?.[0];
  if (!param) return false;
  const bindings = new Set();
  if (param.type === "ObjectPattern") for (const property of param.properties) {
    if (property.type !== "ObjectProperty" || property.key?.name !== "className") continue;
    const binding = property.value.type === "AssignmentPattern" ? property.value.left : property.value;
    if (binding.type === "Identifier") bindings.add(binding.name);
  }
  const propsName = param.type === "Identifier" ? param.name : null;
  const follows = (expression, visited = new Set()) => {
    if (visited.size > 32) return false;
    let reaches = false;
    walk(expression, (node, parent) => {
      if (node.type === "MemberExpression" && node.object?.name === propsName && node.property?.name === "className") reaches = true;
      if (node.type !== "Identifier") return;
      if (parent?.type === "MemberExpression" && parent.property === node && !parent.computed) return;
      if (parent?.type === "ObjectProperty" && parent.key === node && parent.value !== node && !parent.computed) return;
      if (bindings.has(node.name)) { reaches = true; return; }
      if (visited.has(node.name)) return;
      const next = new Set(visited); next.add(node.name);
      walk(component.node.body, (local) => {
        if (local.type === "VariableDeclarator" && local.id?.name === node.name && local.init && follows(local.init, next)) reaches = true;
      });
    });
    return reaches;
  };
  return opening.attributes.some((attribute) => (
    attribute.type === "JSXAttribute" && attribute.name?.name === "className" && follows(attribute.value)
    || attribute.type === "JSXSpreadAttribute" && propsName && attribute.argument?.type === "Identifier" && attribute.argument.name === propsName
  ));
}

function resolveRelative(from, specifier) {
  const base = resolve(dirname(from), specifier);
  const candidates = extname(base) ? [base] : [base, ...EXTENSIONS.map((extension) => `${base}${extension}`), ...EXTENSIONS.map((extension) => resolve(base, `index${extension}`))];
  return candidates.find((candidate) => existsSync(candidate)) ?? null;
}

function exportedBy(ast, component, imported, localName) {
  let result = false;
  walk(ast, (node) => {
    if (node.type !== "ExportDefaultDeclaration" && node.type !== "ExportNamedDeclaration") return;
    if (imported === "default" && node.type === "ExportDefaultDeclaration") {
      const declaration = node.declaration;
      if (declaration?.id?.name === component.name || (declaration?.type === "Identifier" && declaration.name === component.name)) result = true;
    }
    if (imported !== "default" && node.type === "ExportNamedDeclaration") {
      if (node.declaration?.id?.name === imported && component.name === imported) result = true;
      for (const specifier of node.specifiers ?? []) {
        if (specifier.exported?.name === imported && specifier.local?.name === component.name) result = true;
      }
    }
  });
  return result || (component.name === localName && imported === component.name);
}

function instanceReferencesComponent({ root, parser, targetPath, targetAst, component, instanceLocation }) {
  const instancePath = worktreePath(root, instanceLocation.file);
  if (!instancePath) return { error: "invalid_instance" };
  const content = readFileSync(instancePath.absolute, "utf8");
  const ast = parseCode(parser, content, instancePath.file);
  const opening = findOpening(ast, instanceLocation.line, instanceLocation.column);
  const tag = jsxName(opening?.name);
  if (!opening || !tag || isIntrinsic(opening)) return { error: "invalid_instance" };

  if (instancePath.absolute === targetPath.absolute && tag === component.name) return { path: instancePath, content, ast, opening };
  let valid = false;
  walk(ast, (node) => {
    if (node.type !== "ImportDeclaration" || !node.source?.value?.startsWith(".")) return;
    const importedPath = resolveRelative(instancePath.absolute, node.source.value);
    if (importedPath !== targetPath.absolute) return;
    for (const specifier of node.specifiers) {
      if (specifier.local?.name !== tag) continue;
      const imported = specifier.type === "ImportDefaultSpecifier" ? "default" : specifier.imported?.name;
      if (imported && exportedBy(targetAst, component, imported, tag)) valid = true;
    }
  });
  return valid ? { path: instancePath, content, ast, opening } : { error: "invalid_instance" };
}

function chooseTarget(request, context) {
  const { root, parser, definitionPath, definitionContent, definitionAst, definitionOpening, sourceLocation } = context;
  const component = enclosingComponent(definitionAst, definitionOpening);
  const directScreen = definitionPath.file.endsWith(".screen.tsx") && (component
    ? exportedBy(definitionAst, component, "default", component.name)
    : definitionAst.program.body.some((node) => node.type === "ExportDefaultDeclaration"
      && node.declaration?.start <= definitionOpening.start && node.declaration?.end >= definitionOpening.end));
  if (!isIntrinsic(definitionOpening)) return { error: "source_not_found" };
  if (request.scope === "instance" && request.instanceSource) {
    if (!component) return { error: "invalid_instance" };
    const instanceLocation = parseLocation(request.instanceSource);
    if (!instanceLocation) return { error: "invalid_instance" };
    const reference = instanceReferencesComponent({ root, parser, targetPath: definitionPath, targetAst: definitionAst, component, instanceLocation });
    if (reference.error) return reference;
    return {
      ...reference,
      component,
      acceptsClassName: componentAcceptsClassName(component, definitionAst),
      forwardsClassName: classNameReachesOpening(component, definitionOpening),
      definition: { path: definitionPath, opening: definitionOpening, sourceLocation },
      // `selectionSource` identifies the DOM definition to reselect after HMR.
      // `source` identifies the source file that Rust should write for this scope.
      selectionSource: locationString(definitionPath.file, definitionOpening),
      source: locationString(reference.path.file, reference.opening),
      shared: true,
    };
  }
  return {
    path: definitionPath,
    content: definitionContent,
    ast: definitionAst,
    opening: definitionOpening,
    component,
    acceptsClassName: true,
    missingInstance: request.scope === "instance" && !directScreen,
    definition: { path: definitionPath, opening: definitionOpening, sourceLocation },
    selectionSource: locationString(definitionPath.file, definitionOpening),
    source: locationString(definitionPath.file, definitionOpening),
    // A component-level edit affects every callsite. A plain/default instance
    // request edits the selected DOM definition and is not a callsite target.
    shared: request.scope === "component" || !directScreen,
  };
}

function arbitrary(value, typeHint) {
  const trimmed = String(value).trim();
  // Tailwind arbitrary values use underscores for literal spaces. Quotes,
  // brackets, entities, and control characters would produce a malformed or
  // surprising source edit, so keep this writer deliberately narrow.
  if (!trimmed || /[\[\]"'&<>\r\n\t]/.test(trimmed) || !/^[#(),.%+*/:a-zA-Z0-9_\-\s]+$/.test(trimmed)) return null;
  const normalized = trimmed.replace(/\s+/g, "_");
  const explicitType = /^(?:var|calc)\(/.test(normalized)
    || /^(?:inherit|initial|unset|revert(?:-layer)?|xx-small|x-small|small|medium|large|x-large|xx-large|xxx-large|smaller|larger)$/.test(normalized);
  const typed = typeHint && explicitType ? `${typeHint}:${normalized}` : normalized;
  return `[${typed}]`;
}

function propertySpec(property, value) {
  const normalized = String(property ?? "").replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`).toLowerCase();
  const suffix = arbitrary(value);
  if (!suffix) return { unsafe: true };
  const spacing = { padding: "p", "padding-top": "pt", "padding-right": "pr", "padding-bottom": "pb", "padding-left": "pl", margin: "m", "margin-top": "mt", "margin-right": "mr", "margin-bottom": "mb", "margin-left": "ml" };
  if (spacing[normalized]) return { family: normalized.startsWith("padding") ? "padding" : "margin", token: `${spacing[normalized]}-${suffix}`, direction: spacing[normalized], property: normalized };
  if (normalized === "width") return { family: "width", token: `w-${suffix}` };
  if (normalized === "height") return { family: "height", token: `h-${suffix}` };
  if (normalized === "border-radius") return { family: "radius", token: `rounded-${suffix}` };
  if (normalized === "background-color") return { family: "background", token: `bg-${arbitrary(value, "color")}` };
  if (normalized === "color") return { family: "color", token: `text-${arbitrary(value, "color")}` };
  if (normalized === "font-size") return { family: "font-size", token: `text-${arbitrary(value, "length")}` };
  if (normalized === "font-weight") {
    if (value.trim() === "normal" || value.trim() === "bold") return { family: "font-weight", token: `font-${value.trim()}` };
    return { family: "font-weight", token: /^\d+$/.test(value.trim()) ? `font-${suffix}` : `font-[number:${suffix.slice(1, -1)}]` };
  }
  return null;
}

function inlineOverrides(opening, property) {
  const style = opening.attributes.find((attribute) => attribute.type === "JSXAttribute" && attribute.name?.name === "style");
  const object = style?.value?.expression;
  if (object?.type !== "ObjectExpression") return false;
  const wanted = property.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`).toLowerCase();
  return object.properties.some((entry) => {
    if (entry.type !== "ObjectProperty" || entry.computed || entry.value?.type === "NullLiteral") return false;
    const key = String(entry.key?.name ?? entry.key?.value ?? "").replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`).toLowerCase();
    return key === wanted || (key === "padding" && wanted.startsWith("padding-")) || (key === "margin" && wanted.startsWith("margin-"))
      || key === "background" && wanted === "background-color" || wanted === "border-radius" && /^border-.*-radius$/.test(key);
  });
}

function isUnprefixed(token) {
  let brackets = 0;
  let escaped = false;
  for (const character of token) {
    if (escaped) {
      escaped = false;
      continue;
    }
    if (character === "\\") {
      escaped = true;
      continue;
    }
    if (character === "[") brackets += 1;
    else if (character === "]") brackets = Math.max(0, brackets - 1);
    else if (character === ":" && brackets === 0) return false;
  }
  return true;
}

function paddingToken(token, prefix) {
  const match = token.match(new RegExp(`^(-?)${prefix}([trblxy]?)-(.+)$`));
  return match ? { sign: match[1], direction: `${prefix}${match[2]}`, value: match[3] } : null;
}

function utilityModifier(token) {
  let depth = 0;
  for (let index = 0; index < token.length; index++) {
    if (token[index] === "[") depth++;
    if (token[index] === "]") depth--;
    if (token[index] === "/" && depth === 0) return { base: token.slice(0, index), modifier: token.slice(index + 1) };
  }
  return { base: token, modifier: null };
}

function textIsFontSize(token) {
  token = utilityModifier(token).base;
  const value = token.slice(5);
  if (/^(xs|sm|base|lg|xl|[2-9]xl)$/.test(value)) return true;
  if (!/^\[.*\]$/.test(value)) return false;
  const arbitraryValue = value.slice(1, -1);
  if (/^(?:color:|color(?:-mix)?\(|#|rgb\(|hsl\(|hwb\(|lab\(|lch\(|oklab\(|oklch\()/.test(arbitraryValue)) return false;
  if (/^(?:xx-small|x-small|small|medium|large|x-large|xx-large|xxx-large|smaller|larger)$/.test(arbitraryValue)) return true;
  return /^length:/.test(arbitraryValue) || /(?:^|[^a-z])(?:px|rem|em|%|vw|vh|ch|ex|cm|mm|in|pt|pc)(?:$|[^a-z])/.test(arbitraryValue)
    || /^calc\(/.test(arbitraryValue);
}

const TEXT_NON_COLOR = new Set([
  "text-left", "text-center", "text-right", "text-justify", "text-start", "text-end",
  "text-ellipsis", "text-clip", "text-wrap", "text-nowrap", "text-balance", "text-pretty",
  "text-visible", "text-hidden", "text-indent-0", "text-decoration-none",
]);

const TEXT_NON_COLOR_PREFIX = /^text-(?:opacity-|decoration-|underline-offset-|indent-|shadow|transform|case-|truncate$)/;
const BACKGROUND_NON_COLOR = /^bg-(?:none|cover|contain|fixed|local|scroll|clip-|origin-|center$|top$|bottom$|left$|right$|repeat|no-repeat|gradient-to-|opacity-|blend-|auto$)|^bg-\[(?:url\(|image\(|linear-gradient\(|radial-gradient\(|conic-gradient\(|length:|position:|size:)/;

function isColorToken(token) {
  token = utilityModifier(token).base;
  if (!token.startsWith("text-") || TEXT_NON_COLOR.has(token) || TEXT_NON_COLOR_PREFIX.test(token) || textIsFontSize(token)) return false;
  const value = token.slice(5);
  return /^(?:\[(?:color:|color(?:-mix)?\(|#|rgb\(|hsl\(|hwb\(|lab\(|lch\(|oklab\(|oklch\(|var\(|--)[^\]]*\]|\(--[^)]+\)|(?:inherit|current|transparent|black|white|[A-Za-z][\w-]*))$/.test(value);
}

function isBackgroundColorToken(token) {
  token = utilityModifier(token).base;
  if (!token.startsWith("bg-") || BACKGROUND_NON_COLOR.test(token)) return false;
  const value = token.slice(3);
  return /^(?:\[(?:color:|color(?:-mix)?\(|#|rgb\(|hsl\(|hwb\(|lab\(|lch\(|oklab\(|oklch\(|var\(|--)[^\]]*\]|\(--[^)]+\)|(?:inherit|current|transparent|black|white|[A-Za-z][\w-]*))$/.test(value);
}

function isFamilyToken(token, spec) {
  if (!isUnprefixed(token)) return false;
  if (spec.family === "padding") return Boolean(paddingToken(token, "p"));
  if (spec.family === "margin") return Boolean(paddingToken(token, "m"));
  if (spec.family === "width") return /^w-/.test(token);
  if (spec.family === "height") return /^h-/.test(token);
  if (spec.family === "radius") return /^rounded(?:-|$)/.test(token);
  if (spec.family === "background") return isBackgroundColorToken(token);
  if (spec.family === "color") return isColorToken(token);
  if (spec.family === "font-size") return /^text-/.test(token) && textIsFontSize(token);
  if (spec.family === "font-weight") return /^font-(thin|extralight|light|normal|medium|semibold|bold|extrabold|black)$/.test(token)
    || /^font-\[(?:\d+|number:[^\]]+|var\(--[\w-]+\))\]$/.test(token);
  return false;
}

function expandDirectional(token, spec) {
  const prefix = spec.family === "padding" ? "p" : "m";
  const item = paddingToken(token, prefix);
  if (!item || spec.direction === prefix) return [token];
  const value = item.value;
  if (item.direction === prefix) return [`${item.sign}${prefix}t-${value}`, `${item.sign}${prefix}r-${value}`, `${item.sign}${prefix}b-${value}`, `${item.sign}${prefix}l-${value}`];
  if (item.direction === `${prefix}x`) return [`${item.sign}${prefix}r-${value}`, `${item.sign}${prefix}l-${value}`];
  if (item.direction === `${prefix}y`) return [`${item.sign}${prefix}t-${value}`, `${item.sign}${prefix}b-${value}`];
  return [token];
}

function applyClassValue(current, spec) {
  const parts = current.split(/(\s+)/);
  const directional = spec.family === "padding" || spec.family === "margin";
  const familyPrefix = spec.family === "padding" ? "p" : "m";
  let changed = false;
  const result = [];
  let inserted = false;
  for (const part of parts) {
    if (!part || /^\s+$/.test(part)) {
      result.push(part);
      continue;
    }
    if (!isFamilyToken(part, spec)) {
      result.push(part);
      continue;
    }
    if (spec.family === "font-size" && utilityModifier(part).modifier) {
      if (!inserted) { result.push(spec.token); inserted = true; }
      result.push(` leading-${utilityModifier(part).modifier}`);
      changed = true;
      continue;
    }
    if (directional && spec.direction !== familyPrefix) {
      const expanded = expandDirectional(part, spec);
      if (expanded.length > 1) {
        const nextTokens = expanded.flatMap((token) => {
          if (!token.replace(/^-/, "").startsWith(`${spec.direction}-`)) return [token];
          if (inserted) return [];
          inserted = true;
          return [spec.token];
        });
        result.push(nextTokens.join(" "));
        changed = true;
        continue;
      }
      if (part.replace(/^-/, "").startsWith(`${spec.direction}-`)) {
        if (!inserted) {
          result.push(spec.token);
          inserted = true;
        } else if (/^\s+$/.test(result.at(-1) ?? "")) {
          // The removed override's separator belonged to that token. Keeping it
          // after a broad utility expansion would manufacture new whitespace.
          result.pop();
        }
        changed = true;
        continue;
      }
      result.push(part);
      continue;
    }
    if (!inserted) {
      result.push(spec.token);
      inserted = true;
    } else if (/^\s+$/.test(result.at(-1) ?? "")) {
      // This separator only separated a same-family token being removed.
      result.pop();
    }
    changed = true;
  }
  if (!inserted) {
    const separator = current && !/\s$/.test(current) ? " " : "";
    result.push(separator, spec.token);
    changed = true;
  }
  return changed ? result.join("") : current;
}

function editOpening(content, opening, nextClass) {
  const attribute = classAttribute(opening);
  const state = staticClass(attribute);
  if (attribute && state.kind !== "static") return { error: "dynamic_classname" };
  if (hasSpread(opening)) return { error: "dynamic_classname" };
  if (state.literal) {
    const original = content.slice(state.literal.start, state.literal.end);
    const quote = original.startsWith("'") ? "'" : '"';
    // JSX string attributes retain raw whitespace and entities. We only change
    // complete utility tokens, so leave every other byte exactly as authored.
    const replacement = `${quote}${nextClass}${quote}`;
    return {
      content: content.slice(0, state.literal.start) + replacement + content.slice(state.literal.end),
      editStart: state.literal.start,
      editEnd: state.literal.end,
      delta: replacement.length - (state.literal.end - state.literal.start),
    };
  }
  const insertAt = opening.name.end;
  const insertion = ` className="${nextClass}"`;
  return {
    content: content.slice(0, insertAt) + insertion + content.slice(insertAt),
    editStart: insertAt,
    editEnd: insertAt,
    delta: insertion.length,
  };
}

function resultFor(target, content, hash, editable, reason) {
  const state = staticClass(classAttribute(target.opening));
  return {
    ok: true,
    file: target.path.file,
    line: target.opening.loc.start.line,
    column: target.opening.loc.start.column,
    hash,
    editable,
    ...(reason ? { reason } : {}),
    shared: target.shared,
    source: target.source,
    selectionSource: target.selectionSource,
    className: typeof state.value === "string" ? state.value.slice(0, 4096) : "",
    kind: classKind(target.opening),
  };
}

function run(request) {
  if (!request || typeof request !== "object" || !["inspect", "apply"].includes(request.operation) || !["instance", "component", undefined].includes(request.scope)) return fail("invalid_request");
  request = { ...request, scope: request.scope ?? "instance" };
  const root = typeof request.root === "string" ? request.root : null;
  const sourceLocation = parseLocation(request.source);
  if (!root || !sourceLocation) return fail("invalid_request");
  let definitionPath;
  try { definitionPath = worktreePath(root, sourceLocation.file); } catch { return fail("path_outside_worktree"); }
  if (!definitionPath) return fail("path_outside_worktree");
  let parser;
  let definitionContent;
  let definitionAst;
  try {
    parser = parserFor(root);
    definitionContent = readFileSync(definitionPath.absolute, "utf8");
    definitionAst = parseCode(parser, definitionContent, definitionPath.file);
  } catch (error) { return fail("source_not_found", error instanceof Error ? error.message : REASON_MESSAGES.source_not_found); }
  const definitionOpening = findOpening(definitionAst, sourceLocation.line, sourceLocation.column);
  if (!definitionOpening) return fail("source_not_found");
  let target;
  try {
    target = chooseTarget(request, { root, parser, definitionPath, definitionContent, definitionAst, definitionOpening, sourceLocation });
  } catch (error) { return fail("invalid_instance", error instanceof Error ? error.message : REASON_MESSAGES.invalid_instance); }
  if (target.error) return fail(target.error);
  const originalHash = digest(target.content);
  const kind = classKind(target.opening);
  if (request.operation === "inspect") {
    const reason = target.missingInstance ? "instance_source_missing" : !target.acceptsClassName ? "no_classname_prop" : target.forwardsClassName === false ? "no_classname_target" : kind === "dynamic" ? "dynamic_classname" : undefined;
    return resultFor(target, target.content, originalHash, !reason, reason);
  }
  if (typeof request.property !== "string" || typeof request.value !== "string") return fail("invalid_request");
  if (request.expectedHash !== originalHash) return fail("stale_file");
  if (target.missingInstance) return fail("instance_source_missing", "The component callsite could not be established. Reselect or explicitly choose component scope.");
  if (!target.acceptsClassName) return fail("no_classname_prop");
  if (target.forwardsClassName === false) return fail("no_classname_target", "The className prop does not reach the selected DOM element. Explicitly choose component scope to change this element.");
  if (kind === "dynamic") return fail("dynamic_classname");
  if (inlineOverrides(target.definition.opening, request.property)) return fail("inline_style_override", "An inline style controls this property. Request a source change in the task conversation.");
  const spec = propertySpec(request.property, request.value);
  if (spec?.unsafe) return fail("unsafe_value");
  if (!spec) return fail("unsupported_property");
  const nextClass = applyClassValue(rawClassValue(target.content, target.opening), spec);
  const edited = editOpening(target.content, target.opening, nextClass);
  if (edited.error) return fail(edited.error);
  const proposedAst = parseCode(parser, edited.content, target.path.file);
  const proposedOpening = findOpeningAtStart(proposedAst, target.opening.start);
  let selectionSource = target.selectionSource;
  if (target.path.absolute === target.definition.path.absolute) {
    const definitionStart = target.definition.opening.start + (edited.editStart <= target.definition.opening.start ? edited.delta : 0);
    const proposedDefinition = findOpeningAtStart(proposedAst, definitionStart);
    if (!proposedDefinition) return fail("source_not_found");
    selectionSource = locationString(target.definition.path.file, proposedDefinition);
  }
  if (!proposedOpening) return fail("source_not_found");
  const proposed = { ...target, opening: proposedOpening, selectionSource };
  return {
    ...resultFor(proposed, edited.content, digest(edited.content), true),
    previousHash: originalHash,
    content: edited.content,
  };
}

let input = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => { input += chunk; });
process.stdin.on("end", () => {
  try {
    process.stdout.write(`${JSON.stringify(run(JSON.parse(input)))}\n`);
  } catch (error) {
    process.stdout.write(`${JSON.stringify(fail("invalid_request", error instanceof Error ? error.message : REASON_MESSAGES.invalid_request))}\n`);
  }
});
