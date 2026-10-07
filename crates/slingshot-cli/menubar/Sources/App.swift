import AppKit
import SwiftUI

@main
struct SlingshotApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        MenuBarExtra {
            PopoverView(watcher: delegate.watcher)
        } label: {
            MenuBarLabel(watcher: delegate.watcher)
        }
        .menuBarExtraStyle(.window)
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    let watcher = Watcher()
    private let notifier = Notifier()

    func applicationDidFinishLaunching(_ notification: Notification) {
        if CommandLine.arguments.contains("--remove") {
            LoginItem.set(false)
            NSApplication.shared.terminate(nil)
            return
        }
        notifier.setUp()
        watcher.onNotice = { [notifier] notice in notifier.post(notice) }
        watcher.start()
        LoginItem.enable()
    }

    /// `slingshot menubar` opens the app again after saving a new program path or Agent.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        watcher.start()
        return false
    }

    func applicationWillTerminate(_ notification: Notification) {
        watcher.stop()
    }
}

/// Only the logo lives in the menu bar. The numbers are one click away, in the panel.
struct MenuBarLabel: View {
    let watcher: Watcher

    var body: some View {
        Image(nsImage: Self.logo)
            .opacity(watcher.status?.online == true ? 1 : 0.5)
    }

    /// The logo's shape at menu bar size, scaled without smoothing so its pixels stay sharp. As a
    /// template image, macOS draws it in the menu bar's own color, white or black, like other icons.
    /// A bundle without the logo falls back to a system symbol rather than an empty space.
    private static let logo: NSImage = {
        guard let source = Bundle.main.image(forResource: "Logo") else {
            return NSImage(systemSymbolName: "server.rack", accessibilityDescription: "Slingshot") ?? NSImage()
        }
        let logo = NSImage(size: NSSize(width: 18, height: 18), flipped: false) { rect in
            NSGraphicsContext.current?.imageInterpolation = .none
            source.draw(in: rect)
            return true
        }
        logo.isTemplate = true
        return logo
    }()
}
