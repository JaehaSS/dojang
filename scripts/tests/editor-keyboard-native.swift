import AppKit
import WebKit

// AppKit menu routing cannot be exercised by Playwright's WebKit browser.
// This standalone host has no access to Dojang's database or running windows.
final class KeyboardProbe: NSObject, NSApplicationDelegate {
    var window: NSWindow!
    var webview: WKWebView!

    @objc func dispatchSelectAll(_ sender: Any?) {
        webview.evaluateJavaScript(CommandLine.arguments[3])
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1200, height: 800),
                          styleMask: [.titled, .closable], backing: .buffered, defer: false)
        webview = WKWebView(frame: window.contentView!.bounds)
        window.contentView = webview
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)

        let menu = NSMenu()
        let appItem = NSMenuItem(title: "Keyboard Probe", action: nil, keyEquivalent: "")
        appItem.submenu = NSMenu(title: "Keyboard Probe")
        menu.addItem(appItem)
        let editItem = NSMenuItem(title: "Edit", action: nil, keyEquivalent: "")
        let edit = NSMenu(title: "Edit")
        for (title, selector, key) in [("Cut", "cut:", "x"), ("Copy", "copy:", "c"), ("Paste", "paste:", "v")] {
            edit.addItem(NSMenuItem(title: title, action: NSSelectorFromString(selector), keyEquivalent: key))
        }
        if CommandLine.arguments[2] == "predefined" {
            edit.addItem(NSMenuItem(title: "Select All", action: NSSelectorFromString("selectAll:"), keyEquivalent: "a"))
        } else {
            let selectAll = NSMenuItem(title: "Select All", action: #selector(dispatchSelectAll(_:)), keyEquivalent: "a")
            selectAll.target = self
            edit.addItem(selectAll)
        }
        editItem.submenu = edit
        menu.addItem(editItem)
        NSApp.mainMenu = menu
        webview.load(URLRequest(url: URL(string: CommandLine.arguments[1])!))
        Task { @MainActor in
            do {
                for _ in 0..<200 {
                    if (try? await webview.evaluateJavaScript("window.__keyboard?.ready()")) as? Bool == true { break }
                    try await Task.sleep(nanoseconds: 50_000_000)
                }
                var selections: [[String: String]] = []
                var captured = false
                let modes = CommandLine.arguments.count > 4
                    ? CommandLine.arguments[4].components(separatedBy: ",")
                    : ["zoom", "tree", "plain-input", "plain-textarea"]
                for mode in modes {
                    window.makeKeyAndOrderFront(nil)
                    window.makeFirstResponder(webview)
                    let plain = mode.hasPrefix("plain-")
                    let prepare = plain
                        ? "document.getElementById('\(mode)').focus()"
                        : "window.__keyboard.open('\(mode)')"
                    _ = try await webview.evaluateJavaScript(prepare)
                    try await Task.sleep(nanoseconds: 250_000_000)
                    if !plain { _ = try await webview.evaluateJavaScript("window.__keyboard.focus()") }
                    try await Task.sleep(nanoseconds: 100_000_000)
                    let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: .command,
                                                timestamp: ProcessInfo.processInfo.systemUptime,
                                                windowNumber: window.windowNumber, context: nil,
                                                characters: "a", charactersIgnoringModifiers: "a",
                                                isARepeat: false, keyCode: 0)!
                    // Match the responder route: the menu gets first refusal.
                    let handled = NSApp.mainMenu!.performKeyEquivalent(with: event)
                    captured = captured || handled
                    if !handled { NSApp.sendEvent(event) }
                    let up = NSEvent.keyEvent(with: .keyUp, location: .zero, modifierFlags: .command,
                                             timestamp: ProcessInfo.processInfo.systemUptime,
                                             windowNumber: window.windowNumber, context: nil,
                                             characters: "a", charactersIgnoringModifiers: "a",
                                             isARepeat: false, keyCode: 0)!
                    NSApp.sendEvent(up)
                    try await Task.sleep(nanoseconds: 150_000_000)
                    let inspect = plain
                        ? "(() => { const el = document.getElementById('\(mode)'); return { selected: el.value.slice(el.selectionStart, el.selectionEnd), expected: el.value }; })()"
                        : "({ selected: window.__keyboard.snapshot().selected, expected: window.__keyboard.original })"
                    var value = try await webview.evaluateJavaScript(inspect) as! [String: String]
                    value["mode"] = mode
                    selections.append(value)
                }
                let result: [String: Any] = ["menuCaptured": captured, "selections": selections]
                let data = try JSONSerialization.data(withJSONObject: result)
                print(String(data: data, encoding: .utf8)!)
                NSApp.terminate(nil)
            } catch {
                fputs("native keyboard probe failed: \(error)\n", stderr)
                exit(1)
            }
        }
    }
}

let app = NSApplication.shared
let probe = KeyboardProbe()
app.setActivationPolicy(.regular)
app.delegate = probe
app.run()
