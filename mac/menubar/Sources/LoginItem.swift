import Foundation
import ServiceManagement

/// Starting at login is what makes installing once enough, so the app is always a login
/// item. `slingshot menubar --remove` starts the app with `--remove` to take it off.
enum LoginItem {
    static func enable() {
        guard SMAppService.mainApp.status != .enabled else { return }
        set(true)
    }

    static func set(_ on: Bool) {
        do {
            if on {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
        } catch {
            NSLog("Slingshot could not change the login item: \(error.localizedDescription)")
        }
    }
}
