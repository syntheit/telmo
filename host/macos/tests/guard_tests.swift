// Run with tests/run.sh. Covers the pure parts of BluetoothGuard.swift.
import Foundation

func check(_ ok: Bool, _ what: String) {
    if !ok { print("FAIL: \(what)"); exit(1) }
}

@main struct GuardTests {
    static func main() {
        check(BluetoothAddress.parse("aa:bb:cc:dd:ee:ff") == "AA:BB:CC:DD:EE:FF", "lower case colon")
        check(BluetoothAddress.parse("aa-bb-cc-dd-ee-ff") == "AA:BB:CC:DD:EE:FF", "dashes")
        for bad in ["", "AA:BB:CC:DD:EE", "AA:BB:CC:DD:EE:FF:00", "AA:BB:CC:DD:EE:GG", "AABBCCDDEEFF", "A:BB:CC:DD:EE:FF", "AA:BB:CC:DD:EE:FF; rm"] {
            check(BluetoothAddress.parse(bad) == nil, "rejects \(bad)")
        }

        var window = AllowWindow()
        let t0 = Date(timeIntervalSince1970: 1000)
        check(!window.isAllowed("AA:BB:CC:DD:EE:FF", now: t0), "nothing allowed at first")
        window.allow("AA:BB:CC:DD:EE:FF", now: t0)
        check(window.isAllowed("AA:BB:CC:DD:EE:FF", now: t0.addingTimeInterval(19)), "allowed inside the window")
        check(!window.isAllowed("AA:BB:CC:DD:EE:FF", now: t0.addingTimeInterval(21)), "expired")
        check(!window.isAllowed("11:22:33:44:55:66", now: t0), "other address")

        var config = BluetoothGuardConfig()
        config.noAutoConnect = ["AA:BB:CC:DD:EE:FF"]
        check(config.isNoAutoConnect("aa-bb-cc-dd-ee-ff"), "list matches IOBluetooth spelling")
        let decoded = try? JSONDecoder().decode(BluetoothGuardConfig.self, from: Data("{}".utf8))
        check(decoded == BluetoothGuardConfig(), "empty file means defaults")
        print("ok")
    }
}
