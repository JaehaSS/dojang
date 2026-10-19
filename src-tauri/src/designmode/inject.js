// Design Mode 요소 선택기 — Tauri 자식 웹뷰(dev 서버 origin)에 initialization_script로
// 주입되는 순수 브라우저 스크립트다. 번들러/모듈 시스템을 쓰지 않는다(임의 origin의 CSP는
// 페이지 <script>에는 적용되지만, 네이티브 셸이 주입하는 user script는 그 대상이 아니다).
//
// 순수 로직(_internal에 노출된 함수들)은 `inject.test.js`(vitest, jsdom)가 이 파일을 raw
// 텍스트로 읽어 `new Function(source)()`로 현재 global에서 실행한 뒤 검증한다 — 실제
// 웹뷰 주입과 동일한 실행 경로를 그대로 테스트한다.
(function () {
  "use strict";

  // 레이아웃·색·타이포 중심 화이트리스트(~40속성) — Rust `designmode::CSS_WHITELIST`와 동일한
  // 목록을 유지한다(백엔드가 fail-closed로 다시 한번 걸러내므로 여기서는 프롬프트 크기 절약용).
  var CSS_WHITELIST = [
    "display", "position", "top", "right", "bottom", "left", "width", "height",
    "min-width", "min-height", "max-width", "max-height", "margin", "margin-top",
    "margin-right", "margin-bottom", "margin-left", "padding", "padding-top",
    "padding-right", "padding-bottom", "padding-left", "box-sizing", "flex",
    "flex-direction", "flex-wrap", "align-items", "justify-content", "gap",
    "grid-template-columns", "color", "background-color", "border", "border-radius",
    "border-width", "border-style", "border-color", "font-family", "font-size",
    "font-weight", "font-style", "line-height", "letter-spacing", "text-align",
    "text-decoration", "opacity", "box-shadow", "z-index", "overflow"
  ];

  var MAX_OUTER_HTML = 20000;
  var CAPTURE_SCHEME = "praxis-designmode";
  var HIGHLIGHT_ID = "__praxis_designmode_highlight__";

  /** computed style에서 화이트리스트 속성만 추출 — `getPropertyValue`만 있으면 되므로 순수 테스트 가능. */
  function whitelistComputedStyle(computed, whitelist) {
    var out = {};
    for (var i = 0; i < whitelist.length; i++) {
      var prop = whitelist[i];
      var value = computed && computed.getPropertyValue ? computed.getPropertyValue(prop) : "";
      if (value) out[prop] = value;
    }
    return out;
  }

  /** DOMRect 유사 객체 → 순수 데이터. */
  function toBoundingRect(rect) {
    return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
  }

  /** 큰 컨테이너를 실수로 골랐을 때 프롬프트 폭발을 막는다. */
  function truncateHtml(html) {
    if (html.length <= MAX_OUTER_HTML) return html;
    return html.slice(0, MAX_OUTER_HTML) + "\n<!-- …truncated -->";
  }

  /** 요소 → 캡처 페이로드. 필드명은 Rust `ElementCapture`(snake_case, 기존 IPC 컨벤션)와 일치시킨다. */
  function serializeElement(el, computed) {
    return {
      outer_html: truncateHtml(el.outerHTML || ""),
      computed_css: whitelistComputedStyle(computed, CSS_WHITELIST),
      bounding_rect: toBoundingRect(el.getBoundingClientRect())
    };
  }

  /** Rust `on_navigation`이 가로챌 커스텀 스킴 URL. 실제 navigate는 native 쪽에서 항상 취소된다. */
  function buildCaptureUrl(payload) {
    return CAPTURE_SCHEME + ":capture?data=" + encodeURIComponent(JSON.stringify(payload));
  }

  /** hover 하이라이트 박스의 인라인 스타일(순수 함수 — rect만 있으면 됨). */
  function highlightStyleFor(rect) {
    return {
      position: "fixed",
      left: rect.left + "px",
      top: rect.top + "px",
      width: rect.width + "px",
      height: rect.height + "px",
      pointerEvents: "none",
      zIndex: "2147483647",
      background: "rgba(45, 212, 191, 0.18)",
      outline: "2px solid rgba(45, 212, 191, 0.9)"
    };
  }

  var enabled = false;
  var highlightEl = null;

  function ensureHighlightEl() {
    if (highlightEl) return highlightEl;
    highlightEl = document.createElement("div");
    highlightEl.id = HIGHLIGHT_ID;
    document.documentElement.appendChild(highlightEl);
    return highlightEl;
  }

  function clearHighlight() {
    if (highlightEl && highlightEl.parentNode) highlightEl.parentNode.removeChild(highlightEl);
    highlightEl = null;
  }

  function applyHighlight(el) {
    var box = ensureHighlightEl();
    var style = highlightStyleFor(el.getBoundingClientRect());
    for (var key in style) {
      if (Object.prototype.hasOwnProperty.call(style, key)) box.style[key] = style[key];
    }
  }

  function onMouseMove(e) {
    if (!enabled || !e.target || e.target === highlightEl) return;
    applyHighlight(e.target);
  }

  function onClick(e) {
    if (!enabled || !e.target || e.target === highlightEl) return;
    e.preventDefault();
    e.stopPropagation();
    var payload = serializeElement(e.target, window.getComputedStyle(e.target));
    // 선택 모드만 내리고 하이라이트는 화면에 남긴다. 네이티브 스크린샷은 별도 스레드에서
    // 뒤늦게 찍히므로, 그 사이 마우스가 움직여 하이라이트가 다른 요소로 옮겨가면 이미지가
    // 엉뚱한 곳을 가리키게 된다. `setEnabled(false)`는 하이라이트까지 지우므로 쓰지 않는다.
    enabled = false;
    window.location.href = buildCaptureUrl(payload);
  }

  function setEnabled(next) {
    enabled = !!next;
    if (!enabled) clearHighlight();
  }

  /**
   * Design Canvas iframe용 선택기. 기존 Preview 선택기와 달리 문서를 인자로 받으므로
   * 부모 웹뷰에서 같은 출처 iframe의 렌더 문서에 붙일 수 있다. 클릭 결과의 전달은
   * 호출자가 맡는다. 이 raw 스크립트는 네이티브 여부를 표시하거나 navigate하지 않는다.
   */
  function bindCanvasDocument(doc, onSelection) {
    if (!doc || !doc.documentElement || typeof onSelection !== "function") {
      throw new TypeError("bindCanvasDocument requires a document and selection callback");
    }

    var canvasEnabled = false;
    var canvasHighlight = null;
    var lastSuggestedAncestor = null;
    var lastSelectedElement = null;

    function sourceOf(el) {
      var source = el && el.getAttribute && el.getAttribute("data-praxis-src");
      return source || null;
    }

    function classKindOf(el) {
      var kind = el && el.getAttribute && el.getAttribute("data-praxis-class-kind");
      return kind === "static" || kind === "dynamic" || kind === "none" || kind === "unknown"
        ? kind
        : "unknown";
    }

    function selectionIdentity(el) {
      return {
        source: sourceOf(el),
        instanceSource: instanceSourceOf(el),
        className: el && el.getAttribute ? (el.getAttribute("class") || "") : "",
        classKind: classKindOf(el),
        tagName: el && el.tagName ? el.tagName.toLowerCase() : ""
      };
    }

    // React's development DOM fiber identifies the actual invocation even when
    // a component does not forward data props. Missing internals fail closed.
    function instanceSourceOf(el) {
      if (!el) return null;
      var keys = Object.keys(el);
      var fiberKey = keys.find(function (key) { return key.indexOf("__reactFiber$") === 0; });
      var fiber = fiberKey && el[fiberKey];
      var depth = 0;
      while (fiber && depth++ < 100) {
        if (typeof fiber.type !== "string" && fiber.memoizedProps) {
          var source = fiber.memoizedProps["data-dojang-instance-src"];
          if (typeof source === "string") return source;
        }
        fiber = fiber.return;
      }
      return null;
    }

    function nearestTaggedAncestor(el) {
      var current = el && el.parentElement;
      while (current) {
        if (sourceOf(current)) return current;
        current = current.parentElement;
      }
      return null;
    }

    function serializeCanvasElement(el, suggestedAncestor) {
      var payload = selectionIdentity(el);
      payload.version = 1;
      payload.computedCss = whitelistComputedStyle(doc.defaultView.getComputedStyle(el), CSS_WHITELIST);
      payload.boundingRect = toBoundingRect(el.getBoundingClientRect());
      payload.suggestedAncestor = suggestedAncestor ? selectionIdentity(suggestedAncestor) : null;
      return payload;
    }

    function ensureCanvasHighlight() {
      if (canvasHighlight) return canvasHighlight;
      canvasHighlight = doc.createElement("div");
      canvasHighlight.id = HIGHLIGHT_ID;
      doc.documentElement.appendChild(canvasHighlight);
      return canvasHighlight;
    }

    function clearCanvasHighlight() {
      if (canvasHighlight && canvasHighlight.parentNode) {
        canvasHighlight.parentNode.removeChild(canvasHighlight);
      }
      canvasHighlight = null;
    }

    function applyCanvasHighlight(el) {
      var box = ensureCanvasHighlight();
      var style = highlightStyleFor(el.getBoundingClientRect());
      for (var key in style) {
        if (Object.prototype.hasOwnProperty.call(style, key)) box.style[key] = style[key];
      }
    }

    function select(el, suggestedAncestor) {
      if (!el || el === canvasHighlight) return null;
      lastSuggestedAncestor = suggestedAncestor || null;
      lastSelectedElement = el;
      applyCanvasHighlight(el);
      var payload = serializeCanvasElement(el, lastSuggestedAncestor);
      onSelection(payload);
      return payload;
    }

    function onCanvasMouseMove(event) {
      if (!canvasEnabled || !event.target || event.target === canvasHighlight) return;
      applyCanvasHighlight(event.target);
    }

    function onCanvasClick(event) {
      if (!canvasEnabled || !event.target || event.target === canvasHighlight) return;
      event.preventDefault();
      event.stopPropagation();
      // 한 번 고른 뒤 선택 모드만 내린다. 선택 지점을 계속 보여 주도록 highlight는 남긴다.
      canvasEnabled = false;
      var target = event.target;
      select(target, sourceOf(target) ? null : nearestTaggedAncestor(target));
    }

    doc.addEventListener("mousemove", onCanvasMouseMove, true);
    doc.addEventListener("click", onCanvasClick, true);

    return {
      setEnabled: function (next) {
        canvasEnabled = !!next;
        if (!canvasEnabled) clearCanvasHighlight();
      },
      selectSuggestedAncestor: function () {
        if (!lastSuggestedAncestor) return null;
        return select(lastSuggestedAncestor, null);
      },
      getSelectedElement: function () { return lastSelectedElement; },
      reselectSource: function (source, instanceSource) {
        var elements = Array.prototype.filter.call(doc.querySelectorAll("[data-praxis-src]"), function (el) {
          return sourceOf(el) === source && (!instanceSource || instanceSourceOf(el) === instanceSource);
        });
        if (elements.length !== 1) { clearCanvasHighlight(); lastSelectedElement = null; return false; }
        select(elements[0], null);
        return true;
      },
      dispose: function () {
        canvasEnabled = false;
        doc.removeEventListener("mousemove", onCanvasMouseMove, true);
        doc.removeEventListener("click", onCanvasClick, true);
        clearCanvasHighlight();
        lastSuggestedAncestor = null;
        lastSelectedElement = null;
      }
    };
  }

  document.addEventListener("mousemove", onMouseMove, true);
  document.addEventListener("click", onClick, true);

  window.__praxisDesignMode = {
    setEnabled: setEnabled,
    bindCanvasDocument: bindCanvasDocument,
    isEnabled: function () {
      return enabled;
    },
    // 테스트 훅 — 부작용 없는 순수 함수만 노출한다.
    _internal: {
      whitelistComputedStyle: whitelistComputedStyle,
      toBoundingRect: toBoundingRect,
      truncateHtml: truncateHtml,
      serializeElement: serializeElement,
      buildCaptureUrl: buildCaptureUrl,
      highlightStyleFor: highlightStyleFor,
      CSS_WHITELIST: CSS_WHITELIST
    }
  };
})();
