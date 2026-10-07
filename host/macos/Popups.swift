import Foundation

/// What to run in the popup and how to present it.
struct Launch {
    let executable: String
    let args: [String]
    let large: Bool
    let escapeCloses: Bool
}

enum Popups {
    /// `~/.config/telmo/popups.json`, read on every show so edits apply without a restart:
    /// `{"perf": {"command": ["btop"], "size": "large", "escape": "close"}}`.
    private static func configured() -> [String: [String: Any]] {
        let dir = ProcessInfo.processInfo.environment["XDG_CONFIG_HOME"].flatMap { $0.isEmpty ? nil : $0 }
            ?? NSHomeDirectory() + "/.config"
        let path = dir + "/telmo/popups.json"
        guard let data = FileManager.default.contents(atPath: path),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: [String: Any]] else { return [:] }
        return json
    }

    /// Built-in modules (`telmo-<name>`) first, then popups.json. The reason a lookup failed is the error.
    static func resolve(_ name: String) -> Result<Launch, PopupError> {
        if let exe = ModuleLookup.find(name) {
            return .success(Launch(executable: exe, args: [], large: false, escapeCloses: false))
        }
        guard let entry = configured()[name] else { return .failure(PopupError("module not found: telmo-\(name)")) }
        guard let command = entry["command"] as? [String], let program = command.first else {
            return .failure(PopupError("popup '\(name)' in popups.json needs a non-empty \"command\" list"))
        }
        guard let exe = ModuleLookup.findExecutable(program) else {
            return .failure(PopupError("command not found for popup '\(name)': \(program)"))
        }
        return .success(Launch(executable: exe, args: Array(command.dropFirst()),
                               large: entry["size"] as? String == "large",
                               escapeCloses: entry["escape"] as? String == "close"))
    }
}

struct PopupError: Error {
    let message: String
    init(_ message: String) { self.message = message }
}
