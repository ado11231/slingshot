import Foundation
import Observation

/// Runs `slingshot internal-watch` and keeps what it reports. Closing the helper's input is what stops it, so it never outlives the app.
@MainActor
@Observable
final class Watcher {
    private(set) var status: Status?
    private(set) var lastSeen: Date?
    /// True from Try again until the next answer, so the button can show it is working.
    private(set) var retrying = false
    /// Why there is nothing to show, such as a helper that keeps failing.
    private(set) var problem: String?
    /// Set when the app cannot find the slingshot program: the saved path that is gone, or an
    /// empty string when no path was ever saved. Trying again cannot fix this.
    private(set) var missingProgram: String?

    var onNotice: ((Notice) -> Void)?

    private var process: Process?
    private var input: Pipe?
    private var buffer = Data()
    private var errors = Data()
    private var restarts = 0
    private var stopping = false
    private var started: (program: String, agent: String?)?

    private static let backoff: [TimeInterval] = [2, 5, 15, 30]

    /// Start the helper, or restart it when `slingshot menubar` saved a new program or Agent.
    func start() {
        let defaults = UserDefaults.standard
        let saved = defaults.string(forKey: "slingshotPath")
        guard let program = saved, FileManager.default.isExecutableFile(atPath: program) else {
            missingProgram = saved ?? ""
            return
        }
        missingProgram = nil
        let agent = defaults.string(forKey: "agent")
        if process?.isRunning == true, started?.program == program, started?.agent == agent {
            return
        }
        stop()
        stopping = false
        launch(program: program, agent: agent)
    }

    /// Skip the helper's wait and try the Agent again now.
    func retry() {
        guard let input, process?.isRunning == true else {
            start()
            return
        }
        retrying = true
        try? input.fileHandleForWriting.write(contentsOf: Data("retry\n".utf8))
    }

    func stop() {
        stopping = true
        try? input?.fileHandleForWriting.close()
        process = nil
        input = nil
    }

    private func launch(program: String, agent: String?) {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: program)
        var arguments = ["--color", "never"]
        if let agent { arguments += ["--agent", agent] }
        process.arguments = arguments + ["internal-watch"]

        let input = Pipe()
        let output = Pipe()
        let errorOutput = Pipe()
        process.standardInput = input
        process.standardOutput = output
        process.standardError = errorOutput

        output.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            Task { @MainActor in self?.received(data) }
        }
        errorOutput.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            Task { @MainActor in self?.errors.append(data) }
        }
        process.terminationHandler = { [weak self] _ in
            Task { @MainActor in self?.ended(process) }
        }

        buffer = Data()
        errors = Data()
        do {
            try process.run()
        } catch {
            problem = "Could not start Slingshot at \(program): \(error.localizedDescription)"
            return
        }
        self.process = process
        self.input = input
        started = (program, agent)
    }

    private func received(_ data: Data) {
        buffer.append(data)
        while let newline = buffer.firstIndex(of: UInt8(ascii: "\n")) {
            let line = buffer[buffer.startIndex..<newline]
            buffer.removeSubrange(buffer.startIndex...newline)
            guard let event = decodeEvent(Data(line)) else { continue }
            switch event {
            case .status(let status): apply(status)
            case .notice(let notice): onNotice?(notice)
            case .unsupported(let version):
                problem = "Slingshot speaks format \(version), and this app speaks format \(supportedVersion)."
            }
        }
    }

    private func apply(_ status: Status) {
        if self.status?.agent != status.agent {
            lastSeen = nil
        }
        self.status = status
        problem = nil
        restarts = 0
        retrying = false
        if status.online {
            lastSeen = Date()
        }
    }

    /// The helper exits cleanly when Slingshot was updated, so the new program takes over at
    /// once. Any other exit means something is wrong, so show why and try again.
    private func ended(_ ended: Process) {
        guard ended === process, !stopping else { return }
        if ended.terminationStatus == 0 {
            process = nil
            start()
            return
        }
        let message = String(decoding: errors, as: UTF8.self)
            .split(separator: "\n")
            .last
            .map { $0.replacingOccurrences(of: "✗ ", with: "") }
        problem = message ?? "Slingshot stopped unexpectedly."
        process = nil
        let wait = Self.backoff[min(restarts, Self.backoff.count - 1)]
        restarts += 1
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(wait))
            guard let self, !self.stopping, self.process == nil else { return }
            self.start()
        }
    }
}
