import UserNotifications

/// Posts what the helper decided is worth a notification. The app never decides that itself.
final class Notifier: NSObject, UNUserNotificationCenterDelegate {
    private let center = UNUserNotificationCenter.current()

    func setUp() {
        center.delegate = self
        center.requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }

    func post(_ notice: Notice) {
        let content = UNMutableNotificationContent()
        content.title = notice.title
        if let subtitle = notice.subtitle {
            content.subtitle = subtitle
        }
        content.body = notice.body
        content.threadIdentifier = notice.kind
        if notice.kind != "job_finished" {
            content.sound = .default
        }
        center.add(UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil))
    }

    /// A menu bar app counts as frontmost while its panel is open, so ask for the banner anyway.
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .sound, .list])
    }
}
