import SwiftUI

/// Each resource keeps its own color and icon, so the four sections read apart at a glance.
enum Metric {
    case cpu, ram, gpu, workspace

    var label: String {
        switch self {
        case .cpu: "CPU"
        case .ram: "RAM"
        case .gpu: "GPU"
        case .workspace: "Workspace"
        }
    }

    var icon: String {
        switch self {
        case .cpu: "cpu"
        case .ram: "memorychip"
        case .gpu: "square.3.layers.3d"
        case .workspace: "internaldrive"
        }
    }

    var color: Color {
        switch self {
        case .cpu: .blue
        case .ram: .purple
        case .gpu: .green
        case .workspace: .teal
        }
    }
}

/// VRAM sits inside the GPU section but is a different thing from how busy the GPU is, so it
/// gets its own color.
let vramColor = Color.pink

/// Every value beside a bar uses this, so all the numbers match. Digits keep their natural
/// widths, because fixed width digits leave wide gaps around a 1.
let valueFont = Font.system(size: 11, weight: .semibold, design: .rounded)

/// Healthy values stay plain, as on the command line. Color is for what needs attention.
func valueColor(_ level: Level?) -> Color {
    switch level {
    case .warning: .orange
    case .high: .red
    default: .primary
    }
}

struct PopoverView: View {
    let watcher: Watcher

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            header
            if let status = watcher.status, status.online {
                metrics(status)
            } else {
                offline
            }
            footer
        }
        .padding(16)
        .frame(width: 300)
    }

    private var header: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(watcher.status?.online == true ? Color.green : Color.secondary)
                .frame(width: 8, height: 8)
            Text(watcher.status?.agent.nonEmpty ?? "Slingshot")
                .font(.headline)
            Spacer()
            if watcher.status?.online == true, let path = watcher.status?.path {
                Text("via \(path)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else if let lastSeen = watcher.lastSeen {
                TimelineView(.periodic(from: .now, by: 15)) { context in
                    Text("Last seen \(ago(lastSeen, now: context.date))")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    private func ago(_ date: Date, now: Date) -> String {
        guard now.timeIntervalSince(date) >= 60 else { return "just now" }
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .full
        return formatter.localizedString(for: date, relativeTo: now)
    }

    @ViewBuilder
    private func metrics(_ status: Status) -> some View {
        if let cpu = status.cpu {
            MetricSection(metric: .cpu, value: String(format: "%.0f%%", cpu.percent), level: cpu.level) {
                UsageBar(fraction: cpu.percent / 100, color: Metric.cpu.color)
            }
        }
        if let memory = status.memory {
            MetricSection(
                metric: .ram,
                value: "\(capacity(memory.usedMib)) / \(capacity(memory.totalMib))",
                level: memory.level
            ) {
                UsageBar(fraction: memory.fraction, color: Metric.ram.color)
            }
        }
        ForEach(Array(status.gpus.enumerated()), id: \.offset) { index, gpu in
            gpuSection(gpu, title: status.gpus.count > 1 ? "GPU \(index + 1)" : "GPU")
        }
        if let problem = status.gpuProblem {
            MetricSection(metric: .gpu, value: "Unavailable", level: .warning) {
                Text(problem)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        if let workspace = status.workspace {
            MetricSection(
                metric: .workspace,
                value: "\(capacity(workspace.freeMib)) free",
                level: workspace.level
            ) {
                EmptyView()
            }
        }
    }

    private func gpuSection(_ gpu: Gpu, title: String) -> some View {
        MetricSection(
            metric: .gpu,
            title: title,
            detail: gpu.name,
            value: gpu.temperatureC.map { "\($0)°C" } ?? "",
            valueIcon: gpu.temperatureC == nil ? nil : "thermometer.medium",
            level: gpu.temperatureLevel
        ) {
            if let usage = gpu.utilization {
                PartRow(
                    icon: "gauge.with.dots.needle.33percent",
                    label: "Usage",
                    value: String(format: "%.0f%%", usage.percent),
                    level: usage.level,
                    fraction: usage.percent / 100,
                    color: Metric.gpu.color
                )
            }
            if let vram = gpu.vram {
                PartRow(
                    icon: "square.stack.3d.down.right",
                    label: "VRAM",
                    value: "\(capacity(vram.usedMib)) / \(capacity(vram.totalMib))",
                    level: vram.level,
                    fraction: vram.fraction,
                    color: vramColor
                )
            }
        }
    }

    @ViewBuilder
    private var offline: some View {
        if let missing = watcher.missingProgram {
            ProblemView(
                title: "Can't find the slingshot program",
                detail: missing.isEmpty
                    ? "The app has not been told where slingshot is yet."
                    : "It was at \(missing), which no longer exists.",
                fix: Fix(machine: "client", command: "slingshot menubar"),
                agent: nil,
                retrying: false,
                retry: nil
            )
        } else if let problem = watcher.problem {
            ProblemView(
                title: "Slingshot could not start",
                detail: problem,
                fix: Fix(machine: "client", command: "slingshot menubar"),
                agent: nil,
                retrying: watcher.retrying,
                retry: watcher.retry
            )
        } else if let problem = watcher.status?.problem {
            ProblemView(
                title: problem.title,
                detail: problem.detail,
                fix: problem.fix,
                agent: watcher.status?.agent,
                retrying: watcher.retrying,
                retry: watcher.retry
            )
        } else {
            HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                Text("Connecting…")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var footer: some View {
        VStack(spacing: 2) {
            MenuRow(icon: "power", title: "Quit Slingshot") {
                NSApplication.shared.terminate(nil)
            } trailing: {
                Text("⌘Q")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .keyboardShortcut("q")
        }
        .padding(.horizontal, -8)
    }
}

/// A full width row that highlights under the pointer, like a native menu item.
struct MenuRow<Trailing: View>: View {
    let icon: String
    let title: String
    let action: () -> Void
    @ViewBuilder let trailing: Trailing
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 8) {
                Image(systemName: icon)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .frame(width: 18)
                Text(title)
                    .font(.callout)
                Spacer()
                trailing
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .contentShape(Rectangle())
            .background(
                RoundedRectangle(cornerRadius: 6)
                    .fill(hovering ? Color.primary.opacity(0.1) : .clear)
            )
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}

/// One resource: its icon and name, the current value, and whatever graph belongs under it.
struct MetricSection<Content: View>: View {
    let metric: Metric
    var title: String? = nil
    var detail: String? = nil
    let value: String
    var valueIcon: String? = nil
    let level: Level?
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Image(systemName: metric.icon)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(metric.color)
                    .frame(width: 16)
                Text(title ?? metric.label)
                    .font(.caption.weight(.semibold))
                if let detail {
                    Text(detail)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Spacer(minLength: 8)
                HStack(spacing: 3) {
                    if let valueIcon {
                        Image(systemName: valueIcon)
                            .font(.system(size: 9, weight: .semibold))
                    }
                    Text(value)
                        .font(valueFont)
                }
                .foregroundStyle(valueColor(level))
                .layoutPriority(1)
            }
            content
        }
    }
}

extension String {
    var nonEmpty: String? { isEmpty ? nil : self }
}

/// One measured part of a section, such as GPU usage or VRAM, with its own icon and bar.
struct PartRow: View {
    let icon: String
    let label: String
    let value: String
    let level: Level?
    let fraction: Double
    let color: Color

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Image(systemName: icon)
                    .font(.caption2)
                    .foregroundStyle(color)
                    .frame(width: 16)
                Text(label)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Text(value)
                    .font(valueFont)
                    .foregroundStyle(valueColor(level))
            }
            UsageBar(fraction: fraction, color: color)
        }
    }
}

/// What went wrong in plain words, the command that fixes it, and a way to try again now.
struct ProblemView: View {
    let title: String
    let detail: String
    let fix: Fix?
    let agent: String?
    let retrying: Bool
    /// `nil` hides Try again, for problems only the fix can solve.
    let retry: (() -> Void)?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                Text(title)
                    .font(.callout.weight(.semibold))
                    .fixedSize(horizontal: false, vertical: true)
            }
            Text(detail)
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
            if let fix {
                CommandBox(
                    label: fix.machine == "agent" ? "Run on \(agent ?? "the Agent")" : "Run on this machine",
                    command: fix.command
                )
            }
            if let retry {
                Button(action: retry) {
                    HStack(spacing: 6) {
                        if retrying {
                            ProgressView().controlSize(.mini)
                        }
                        Text(retrying ? "Trying…" : "Try again")
                    }
                    .frame(maxWidth: .infinity)
                }
                .controlSize(.regular)
                .disabled(retrying)
            }
        }
    }
}

/// A command shown as code, with a button that copies it.
struct CommandBox: View {
    let label: String
    let command: String
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label)
                .font(.caption2)
                .foregroundStyle(.secondary)
            HStack {
                Text(command)
                    .font(.system(.callout, design: .monospaced))
                    .textSelection(.enabled)
                Spacer()
                Button {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(command, forType: .string)
                    copied = true
                    Task {
                        try? await Task.sleep(for: .seconds(1.5))
                        copied = false
                    }
                } label: {
                    Image(systemName: copied ? "checkmark" : "doc.on.doc")
                        .foregroundStyle(copied ? Color.green : Color.secondary)
                }
                .buttonStyle(.borderless)
                .help("Copy")
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(RoundedRectangle(cornerRadius: 6).fill(Color.primary.opacity(0.06)))
        }
    }
}
