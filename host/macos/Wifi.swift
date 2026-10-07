import CoreWLAN
import Foundation
import LocalAuthentication
import Security

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
        case "wifi-forget-stored":
            deleteStored(ssid: String(line.dropFirst("wifi-forget-stored ".count)))
            return "ok"
        default: return "error unknown command: \(line)"
        }
    }

    static func isWifiCommand(_ line: String) -> Bool { line.hasPrefix("wifi-") }

    // MARK: Saved password

    private static let systemKeychain = "/Library/Keychains/System.keychain"

    private static let service = "io.github.syntheit.telmo.wifi"

    /// Touch ID (or the login password) first, then Telmo's own copy in the login keychain. A network Telmo hasn't
    /// stored yet comes from the system keychain (macOS shows its own dialog once) and is saved for next time.
    /// The reply is "ok <password>" so a password that starts with "error " can't be mistaken for a failure.
    private static func password(ssid: String) -> String {
        guard authenticate(reason: "show the password for \(ssid)") else { return "error Cancelled." }
        if let data = storedData(ssid: ssid) { return "ok " + String(decoding: data, as: UTF8.self) }
        let reply = keychainPassword(ssid: ssid)
        if reply.hasPrefix("ok ") { _ = store(ssid: ssid, password: String(reply.dropFirst(3))) }
        return reply
    }

    private static func storedQuery(_ ssid: String) -> [CFString: Any] {
        [kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: ssid]
    }

    private static func storedData(ssid: String) -> Data? {
        var result: CFTypeRef?
        let query = storedQuery(ssid).merging([kSecReturnData: true]) { $1 }
        return SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess ? result as? Data : nil
    }

    private static func hasStored(ssid: String) -> Bool {
        SecItemCopyMatching(storedQuery(ssid) as CFDictionary, nil) == errSecSuccess
    }

    /// Save or replace Telmo's copy. Nothing else ever sees the password.
    @discardableResult
    private static func store(ssid: String, password: String) -> Bool {
        deleteStored(ssid: ssid)
        let item = storedQuery(ssid).merging([kSecValueData: Data(password.utf8)]) { $1 }
        return SecItemAdd(item as CFDictionary, nil) == errSecSuccess
    }

    private static func deleteStored(ssid: String) {
        SecItemDelete(storedQuery(ssid) as CFDictionary)
    }

    /// Called on an IPC worker thread; the policy callback arrives elsewhere, so wait for it.
    private static func authenticate(reason: String) -> Bool {
        let semaphore = DispatchSemaphore(value: 0)
        var allowed = false
        LAContext().evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason) { success, _ in
            allowed = success
            semaphore.signal()
        }
        semaphore.wait()
        return allowed
    }

    private static func keychainPassword(ssid: String) -> String {
        var keychain: SecKeychain?
        let opened = SecKeychainOpen(systemKeychain, &keychain)
        guard opened == errSecSuccess, let keychain else { return "error Couldn't open the system keychain." }
        let query: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: "AirPort",
            kSecAttrAccount: ssid,
            kSecReturnData: true,
            kSecMatchSearchList: [keychain],
        ]
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        return passwordReply(status: status, data: result as? Data, ssid: ssid)
    }

    static func passwordReply(status: OSStatus, data: Data?, ssid: String) -> String {
        switch status {
        case errSecSuccess:
            guard let data else { return "error Couldn't read the keychain item." }
            return "ok " + String(decoding: data, as: UTF8.self)
        case errSecItemNotFound: return "error No saved password for \(ssid)."
        case errSecAuthFailed, errSecUserCanceled, errSecInteractionNotAllowed: return "error Cancelled."
        default: return "error Couldn't read the keychain (code \(status))."
        }
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
            if !password.isEmpty { store(ssid: ssid, password: password) }
            return "ok"
        } catch let error as NSError {
            if !password.isEmpty && wrongPasswordCodes.contains(error.code) { return "error Wrong password for \(ssid)." }
            if wrongPasswordCodes.contains(error.code) && hasStored(ssid: ssid) {
                deleteStored(ssid: ssid)
                return "error Saved password for \(ssid) no longer works. Enter it again."
            }
            if password.isEmpty && security(network) != "open" && wrongPasswordCodes.contains(error.code) { return "error Enter the password for \(ssid)." }
            return "error Couldn't join \(ssid): \(error.localizedDescription)"
        }
    }
}
