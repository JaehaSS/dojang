import AppKit
import WebKit

// Standalone WKWebView host: no access to Dojang's database or running windows.
final class ImageProbe: NSObject, NSApplicationDelegate {
    var window: NSWindow!
    var webview: WKWebView!

    @MainActor
    func waitFor(_ expression: String) async throws {
        for _ in 0..<160 {
            if (try? await webview.evaluateJavaScript(expression)) as? Bool == true { return }
            try await Task.sleep(nanoseconds: 50_000_000)
        }
        throw NSError(domain: "ImageProbe", code: 1, userInfo: [NSLocalizedDescriptionKey: "Timed out: \(expression)"])
    }

    @MainActor
    func snapshot(_ groups: Int, windowId: String, stage: String) async throws -> [String: Any] {
        try await waitFor("(() => { const imgs = [...document.querySelectorAll('img')]; return imgs.length === \(groups * 5) && imgs.every(img => img.complete && img.naturalWidth > 0); })()")
        let expression = #"({ images: [...document.querySelectorAll('img')].map(img => ({ alt: img.alt, width: img.naturalWidth, src: img.getAttribute('src') })), missing: [...document.querySelectorAll('span')].filter(el => el.textContent === '[이미지를 불러오지 못했습니다: missing]').length, tabs: document.querySelectorAll('[data-tab-path]').length })"#
        var value = try await webview.evaluateJavaScript(expression) as! [String: Any]
        value["windowId"] = windowId
        value["stage"] = stage
        value["groups"] = groups
        return value
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1100, height: 800),
                          styleMask: [.titled], backing: .buffered, defer: false)
        webview = WKWebView(frame: window.contentView!.bounds)
        window.contentView = webview
        Task { @MainActor in
            do {
                var results: [[String: Any]] = []
                for windowId in ["main", "editor"] {
                    webview.load(URLRequest(url: URL(string: CommandLine.arguments[1] + "?window=" + windowId)!))
                    try await waitFor("window.__images?.windowId === '\(windowId)'")
                    results.append(try await snapshot(1, windowId: windowId, stage: "initial"))
                    _ = try await webview.evaluateJavaScript(#"[...document.querySelectorAll('button')].find(el => el.textContent === '줌아웃 파일 열기').click()"#)
                    try await waitFor(#"!!document.querySelector('[data-tab-path="docs/zoom.md"].bg-bg')"#)
                    results.append(try await snapshot(1, windowId: windowId, stage: "zoom"))
                    _ = try await webview.evaluateJavaScript(#"document.querySelector('[aria-label="오른쪽으로 분할"]').click()"#)
                    results.append(try await snapshot(2, windowId: windowId, stage: "split"))
                }
                let data = try JSONSerialization.data(withJSONObject: results)
                print(String(data: data, encoding: .utf8)!)
                NSApp.terminate(nil)
            } catch {
                fputs("native image probe failed: \(error)\n", stderr)
                exit(1)
            }
        }
    }
}

let app = NSApplication.shared
let probe = ImageProbe()
app.setActivationPolicy(.accessory)
app.delegate = probe
app.run()
