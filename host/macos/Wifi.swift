import CoreWLAN
import Foundation

/// Wi-Fi scanning and joining. CoreWLAN only reveals SSIDs to a process that holds the Location grant,
/// and the grant belongs to this app, not to the telmo-net child, so the child asks us. These run on IPC
/// worker threads, never on the main thread.
enum Wifi {
    private static let wrongPasswordCodes: Set<Int> = [-3924, -3925, -3912, -3900]

    static func handle(_ line: String) -> String {
        switch line.split(separator: " ", maxSplits: 1).first {
        case "wifi-scan": return listing(scan: true)
        case "wifi-cached": return listing(scan: false)
        case "wifi-join":
            let args = line.dropFirst("wifi-join ".count).split(separator: "\t", maxSplits: 1, omittingEmptySubsequences: false)
            guard let ssid = args.first, !ssid.isEmpty else { return "error Missing network name." }
            return join(ssid: String(ssid), password: args.count > 1 ? String(args[1]) : "")
        case "wifi-password":
            let ssid = String(line.dropFirst("wifi-password ".count))
            guard !ssid.isEmpty, ssid != "wifi-password" else { return "error Missing network name." }
            return password(ssid: ssid)
        default: return "error unknown command: \(line)"
        }
    }

    static func isWifiCommand(_ line: String) -> Bool { line.hasPrefix("wifi-") }

    // MARK: Saved password

    private static let systemKeychain = "/Library/Keychains/System.keychain"

    /// Wi-Fi passwords live in the system keychain. sudo (Touch ID through pam_tid) is the gate; with no tty it can't fall back to
    /// a terminal prompt. If Touch ID isn't set up for sudo, ask `security` directly, which shows the keychain dialog.
    /// The reply is "ok <password>" so a password that starts with "error " can't be mistaken for a failure.
    private static func password(ssid: String) -> String {
        let viaSudo = run("/usr/bin/sudo", passwordArguments(ssid: ssid, sudo: true))
        if let reply = passwordReply(viaSudo, ssid: ssid) { return reply }
        let direct = run("/usr/bin/security", passwordArguments(ssid: ssid, sudo: false))
        return passwordReply(direct, ssid: ssid) ?? "error Couldn't read the keychain: \(direct.stderr.trimmingCharacters(in: .whitespacesAndNewlines))"
    }

    static func passwordArguments(ssid: String, sudo: Bool) -> [String] {
        let find = ["find-generic-password", "-wa", ssid]
        return sudo ? ["/usr/bin/security"] + find + [systemKeychain] : find
    }

    /// nil means "sudo has no way to authenticate here, try the plain keychain dialog".
    private static func passwordReply(_ out: (status: Int32, stdout: String, stderr: String), ssid: String) -> String? {
        if out.status == 0 { return "ok " + out.stdout.trimmingCharacters(in: CharacterSet(charactersIn: "\n")) }
        let err = out.stderr
        if err.contains("could not be found") { return "error No saved password for \(ssid)." }
        if err.hasPrefix("sudo:") && (err.contains("terminal") || err.contains("askpass") || err.contains("password is required")) { return nil }
        if err.contains("User canceled") || err.contains("denied") || err.contains("authentication") || out.status == 128 { return "error Cancelled." }
        return "error Couldn't read the keychain: \(err.trimmingCharacters(in: .whitespacesAndNewlines))"
    }

    /// argv only, no shell; stdin is /dev/null; killed after 60 seconds.
    private static func run(_ path: String, _ arguments: [String]) -> (status: Int32, stdout: String, stderr: String) {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: path)
        task.arguments = arguments
        task.standardInput = FileHandle.nullDevice
        let out = Pipe(), err = Pipe()
        task.standardOutput = out
        task.standardError = err
        do { try task.run() } catch { return (-1, "", "Couldn't run \(path): \(error.localizedDescription)") }
        let timer = DispatchWorkItem { if task.isRunning { task.terminate() } }
        DispatchQueue.global().asyncAfter(deadline: .now() + 60, execute: timer)
        var errData = Data()
        let reader = DispatchGroup()
        DispatchQueue.global().async(group: reader) { errData = err.fileHandleForReading.readDataToEndOfFile() }
        let outData = out.fileHandleForReading.readDataToEndOfFile()
        reader.wait()
        task.waitUntilExit()
        timer.cancel()
        return (task.terminationStatus, String(decoding: outData, as: UTF8.self), String(decoding: errData, as: UTF8.self))
    }

    // MARK: Listing

    private static func listing(scan: Bool) -> String {
        guard let iface = CWWiFiClient.shared().interface() else { return "error This Mac has no Wi-Fi interface." }
        var found = Set<CWNetwork>()
        if scan {
            do { found = try iface.scanForNetworks(withSSID: nil) } catch {
                return "error Couldn't scan for networks: \(error.localizedDescription)"
            }
        } else {
            found = iface.cachedScanResults() ?? []
        }
        let saved = iface.configuration()?.networkProfiles.compactMap { ($0 as? CWNetworkProfile)?.ssid } ?? []
        let reply: [String: Any] = [
            "current": iface.powerOn() ? (iface.ssid() ?? NSNull()) as Any : NSNull(),
            "networks": entries(found),
            "saved": saved,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: reply, options: [.sortedKeys]) else { return "error Couldn't encode networks." }
        return String(decoding: data, as: UTF8.self)
    }

    private static func entries(_ networks: Set<CWNetwork>) -> [[String: Any]] {
        var best: [String: CWNetwork] = [:]
        for network in networks {
            guard let ssid = network.ssid, !ssid.isEmpty else { continue }
            if let seen = best[ssid], seen.rssiValue >= network.rssiValue { continue }
            best[ssid] = network
        }
        return best.sorted { $0.value.rssiValue > $1.value.rssiValue }.map { ssid, network in
            [
                "ssid": ssid,
                "rssi": network.rssiValue,
                "security": security(network),
                "band": band(network.wlanChannel?.channelBand),
                "channel": network.wlanChannel?.channelNumber ?? 0,
            ]
        }
    }

    private static func security(_ network: CWNetwork) -> String {
        func any(_ list: [CWSecurity]) -> Bool { list.contains { network.supportsSecurity($0) } }
        if any([.wpa3Enterprise, .wpa2Enterprise, .wpaEnterprise, .wpaEnterpriseMixed, .enterprise, .dynamicWEP]) { return "enterprise" }
        if any([.wpa3Personal, .wpa3Transition]) { return "wpa3" }
        if any([.wpa2Personal, .wpaPersonal, .wpaPersonalMixed, .personal]) { return "personal" }
        if network.supportsSecurity(.WEP) { return "wep" }
        if any([.none, .OWE, .oweTransition]) { return "open" }
        return "personal"
    }

    private static func band(_ band: CWChannelBand?) -> String {
        switch band {
        case .band2GHz: return "2"
        case .band6GHz: return "6"
        default: return "5"
        }
    }

    // MARK: Joining

    private static func join(ssid: String, password: String) -> String {
        guard let iface = CWWiFiClient.shared().interface() else { return "error This Mac has no Wi-Fi interface." }
        guard iface.powerOn() else { return "error Wi-Fi is off. Turn it on to join a network." }
        let cached = iface.cachedScanResults()?.first { $0.ssid == ssid }
        guard let network = cached ?? (try? iface.scanForNetworks(withSSID: Data(ssid.utf8)))?.first else {
            return "error \(ssid) isn't in range any more. Rescan and try again."
        }
        do {
            try iface.associate(to: network, password: password.isEmpty ? nil : password)
            return "ok"
        } catch let error as NSError {
            if !password.isEmpty && wrongPasswordCodes.contains(error.code) { return "error Wrong password for \(ssid)." }
            if password.isEmpty && security(network) != "open" && wrongPasswordCodes.contains(error.code) { return "error Enter the password for \(ssid)." }
            return "error Couldn't join \(ssid): \(error.localizedDescription)"
        }
    }
}
