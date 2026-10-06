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
        default: return "error unknown command: \(line)"
        }
    }

    static func isWifiCommand(_ line: String) -> Bool { line.hasPrefix("wifi-") }

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
