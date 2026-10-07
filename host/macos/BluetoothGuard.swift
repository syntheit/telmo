import AppKit
import IOBluetooth

/// The no-auto-connect list in ~/.config/telmo/bluetooth.json. Missing or unreadable files mean the defaults.
struct BluetoothGuardConfig: Codable, Equatable {
    var noAutoConnect: [String] = []

    enum CodingKeys: String, CodingKey {
        case noAutoConnect = "no_auto_connect"
    }

    init() {}

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        noAutoConnect = try c.decodeIfPresent([String].self, forKey: .noAutoConnect) ?? []
    }

    static var path: String {
        ProcessInfo.processInfo.environment["TELMO_BLUETOOTH_CONFIG"] ?? NSHomeDirectory() + "/.config/telmo/bluetooth.json"
    }

    static func load() -> BluetoothGuardConfig {
        guard let data = FileManager.default.contents(atPath: path),
              let config = try? JSONDecoder().decode(BluetoothGuardConfig.self, from: data) else { return BluetoothGuardConfig() }
        return config
    }

    func save() throws {
        let dir = (Self.path as NSString).deletingLastPathComponent
        try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        try encoder.encode(self).write(to: URL(fileURLWithPath: Self.path), options: .atomic)
    }

    func json() -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = .sortedKeys
        return (try? encoder.encode(self)).map { String(decoding: $0, as: UTF8.self) } ?? "{}"
    }

    func isNoAutoConnect(_ address: String) -> Bool {
        guard let address = BluetoothAddress.parse(address) else { return false }
        return noAutoConnect.contains(address)
    }
}

enum BluetoothAddress {
    /// "AA:BB:CC:DD:EE:FF" in either case, or IOBluetooth's "aa-bb-cc-dd-ee-ff". Returns the upper-case colon form.
    static func parse(_ text: String) -> String? {
        let parts = text.split(separator: text.contains("-") ? "-" : ":", omittingEmptySubsequences: false)
        guard parts.count == 6, parts.allSatisfy({ $0.count == 2 && $0.allSatisfy(\.isHexDigit) }) else { return nil }
        return parts.map { $0.uppercased() }.joined(separator: ":")
    }
}

/// Connections Telmo started itself; those may stay up even for a no-auto-connect device.
struct AllowWindow {
    static let lifetime: TimeInterval = 20
    private var until: [String: Date] = [:]

    mutating func allow(_ address: String, now: Date = Date()) {
        until = until.filter { $0.value > now }
        until[address] = now.addingTimeInterval(Self.lifetime)
    }

    func isAllowed(_ address: String, now: Date = Date()) -> Bool {
        until[address].map { $0 > now } ?? false
    }
}

/// Drops devices the user doesn't want connecting by themselves. Main thread only.
final class BluetoothGuard: NSObject {
    private var allowed = AllowWindow()
    private var connectNotification: IOBluetoothUserNotification?

    func start() {
        connectNotification = IOBluetoothDevice.register(forConnectNotifications: self, selector: #selector(deviceConnected(_:device:)))
    }

    // MARK: Auto-connect

    @objc private func deviceConnected(_ note: IOBluetoothUserNotification, device: IOBluetoothDevice) {
        guard let address = device.addressString.flatMap(BluetoothAddress.parse) else { return }
        guard BluetoothGuardConfig.load().isNoAutoConnect(address), !allowed.isAllowed(address) else { return }
        drop(device, attemptsLeft: 5)
    }

    private func drop(_ device: IOBluetoothDevice, attemptsLeft: Int) {
        guard device.isConnected() else { return }
        device.closeConnection()
        guard attemptsLeft > 1 else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [self] in drop(device, attemptsLeft: attemptsLeft - 1) }
    }

    // MARK: Commands

    /// Replies to the bt-* commands, nil for anything else.
    func handle(_ words: [String]) -> String? {
        switch (words.first, words.count) {
        case ("bt-guard-status", 1): return BluetoothGuardConfig.load().json()
        case ("bt-no-autoconnect", 3):
            guard let address = BluetoothAddress.parse(words[1]), words[1].contains(":") else { return "error invalid address: \(words[1])" }
            guard let on = onOff(words[2]) else { return "error expected on or off" }
            return update { config in
                config.noAutoConnect.removeAll { $0 == address }
                if on { config.noAutoConnect.append(address) }
            }
        case ("bt-allow", 2):
            guard let address = BluetoothAddress.parse(words[1]), words[1].contains(":") else { return "error invalid address: \(words[1])" }
            allowed.allow(address)
            return "ok"
        case ("bt-guard-status", _), ("bt-no-autoconnect", _), ("bt-allow", _):
            return "error wrong arguments: \(words.joined(separator: " "))"
        default: return nil
        }
    }

    private func onOff(_ word: String) -> Bool? {
        switch word {
        case "on": return true
        case "off": return false
        default: return nil
        }
    }

    private func update(_ change: (inout BluetoothGuardConfig) -> Void) -> String {
        var config = BluetoothGuardConfig.load()
        change(&config)
        do { try config.save() } catch { return "error Couldn't save \(BluetoothGuardConfig.path): \(error.localizedDescription)" }
        return config.json()
    }
}
