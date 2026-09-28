import Foundation

/// The line format version this app understands. Matches `watch::event::VERSION` in Rust.
let supportedVersion = 1

enum Level: String, Decodable {
    case good, warning, high
}

struct Percent: Decodable {
    let percent: Double
    let level: Level
}

struct Usage: Decodable {
    let usedMib: UInt64
    let totalMib: UInt64
    let level: Level

    var fraction: Double { totalMib == 0 ? 0 : Double(usedMib) / Double(totalMib) }
}

struct Space: Decodable {
    let freeMib: UInt64
    let level: Level
}

struct Gpu: Decodable {
    let name: String
    let utilization: Percent?
    let temperatureC: UInt32?
    let temperatureLevel: Level?
    let vram: Usage?
}

struct Fix: Decodable {
    /// `agent` for the box, `client` for this machine.
    let machine: String
    let command: String
}

struct Problem: Decodable {
    let title: String
    let detail: String
    let fix: Fix?
}

struct Status: Decodable {
    let agent: String
    let online: Bool
    let path: String?
    let error: String?
    let problem: Problem?
    let cpu: Percent?
    let memory: Usage?
    let workspace: Space?
    let gpus: [Gpu]
    let gpuProblem: String?
}

struct Notice: Decodable {
    let kind: String
    let title: String
    let subtitle: String?
    let body: String
}

enum Event {
    case status(Status)
    case notice(Notice)
    case unsupported(Int)
}

private struct Header: Decodable {
    let version: Int
    let event: String
}

private let decoder: JSONDecoder = {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    return decoder
}()

/// One line from the helper. Lines this version does not understand are skipped.
func decodeEvent(_ line: Data) -> Event? {
    guard let header = try? decoder.decode(Header.self, from: line) else { return nil }
    guard header.version == supportedVersion else { return .unsupported(header.version) }
    switch header.event {
    case "status": return (try? decoder.decode(Status.self, from: line)).map(Event.status)
    case "notice": return (try? decoder.decode(Notice.self, from: line)).map(Event.notice)
    default: return nil
    }
}

/// `18.0 GiB` or `512 MiB`, matching how the command line prints sizes.
func capacity(_ mib: UInt64) -> String {
    mib >= 1024 ? String(format: "%.1f GiB", Double(mib) / 1024) : "\(mib) MiB"
}
