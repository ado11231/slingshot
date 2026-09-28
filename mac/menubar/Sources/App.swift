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

    /// `slingshot menubar` opens the app again after saving a new program path or box.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        watcher.start()
        return false
    }

    func applicationWillTerminate(_ notification: Notification) {
        watcher.stop()
    }
}

/// Only the icon lives in the menu bar. The numbers are one click away, in the panel.
struct MenuBarLabel: View {
    let watcher: Watcher

    var body: some View {
        Image(systemName: "server.rack")
            .opacity(watcher.status?.online == true ? 1 : 0.5)
    }
}
