import AppKit
import Carbon.HIToolbox

/// Global hotkeys via Carbon (needs no permissions).
final class Hotkeys {
    private var handlers: [UInt32: String] = [:]
    private var refs: [EventHotKeyRef] = []
    private let onModule: (String) -> Void

    private static let defaults = ["net": "ctrl+opt+cmd+n", "bt": "ctrl+opt+cmd+b", "sound": "ctrl+opt+cmd+m"]
    private static let modifiers: [String: Int] = ["ctrl": controlKey, "opt": optionKey, "alt": optionKey, "cmd": cmdKey, "shift": shiftKey]
    private static let keyCodes: [Character: Int] = [
        "a": kVK_ANSI_A, "b": kVK_ANSI_B, "c": kVK_ANSI_C, "d": kVK_ANSI_D, "e": kVK_ANSI_E, "f": kVK_ANSI_F,
        "g": kVK_ANSI_G, "h": kVK_ANSI_H, "i": kVK_ANSI_I, "j": kVK_ANSI_J, "k": kVK_ANSI_K, "l": kVK_ANSI_L,
        "m": kVK_ANSI_M, "n": kVK_ANSI_N, "o": kVK_ANSI_O, "p": kVK_ANSI_P, "q": kVK_ANSI_Q, "r": kVK_ANSI_R,
        "s": kVK_ANSI_S, "t": kVK_ANSI_T, "u": kVK_ANSI_U, "v": kVK_ANSI_V, "w": kVK_ANSI_W, "x": kVK_ANSI_X,
        "y": kVK_ANSI_Y, "z": kVK_ANSI_Z, "0": kVK_ANSI_0, "1": kVK_ANSI_1, "2": kVK_ANSI_2, "3": kVK_ANSI_3,
        "4": kVK_ANSI_4, "5": kVK_ANSI_5, "6": kVK_ANSI_6, "7": kVK_ANSI_7, "8": kVK_ANSI_8, "9": kVK_ANSI_9,
    ]

    init(onModule: @escaping (String) -> Void) {
        self.onModule = onModule
        installHandler()
        let bindings = Self.defaults.merging(Self.overrides()) { _, new in new }
        for (module, spec) in bindings.sorted(by: { $0.key < $1.key }) { register(module, spec) }
    }

    /// `~/.config/telmo/hotkeys`: lines like `net = ctrl+opt+cmd+n`, `#` comments.
    private static func overrides() -> [String: String] {
        let path = NSHomeDirectory() + "/.config/telmo/hotkeys"
        guard let text = try? String(contentsOfFile: path, encoding: .utf8) else { return [:] }
        var result: [String: String] = [:]
        for line in text.split(separator: "\n") where !line.hasPrefix("#") {
            let parts = line.split(separator: "=", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
            if parts.count == 2 { result[parts[0]] = parts[1] }
        }
        return result
    }

    private func register(_ module: String, _ spec: String) {
        let spec = spec.lowercased()
        if spec == "none" { return }
        var mods = 0
        var key: Int?
        for part in spec.split(separator: "+").map(String.init) {
            if let mod = Self.modifiers[part] { mods |= mod }
            else if part.count == 1, let code = part.first.flatMap({ Self.keyCodes[$0] }) { key = code }
            else { NSLog("telmo: bad hotkey '\(spec)' for \(module)"); return }
        }
        guard let key, mods != 0 else { NSLog("telmo: bad hotkey '\(spec)' for \(module)"); return }

        let id = UInt32(handlers.count + 1)
        var ref: EventHotKeyRef?
        let status = RegisterEventHotKey(UInt32(key), UInt32(mods), EventHotKeyID(signature: 0x54454C4D, id: id),
                                         GetApplicationEventTarget(), 0, &ref)
        guard status == noErr, let ref else { NSLog("telmo: cannot register hotkey '\(spec)' (\(status))"); return }
        refs.append(ref)
        handlers[id] = module
    }

    private func installHandler() {
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        InstallEventHandler(GetApplicationEventTarget(), { _, event, userData in
            guard let event, let userData else { return OSStatus(eventNotHandledErr) }
            var id = EventHotKeyID()
            GetEventParameter(event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID),
                              nil, MemoryLayout<EventHotKeyID>.size, nil, &id)
            let hotkeys = Unmanaged<Hotkeys>.fromOpaque(userData).takeUnretainedValue()
            if let module = hotkeys.handlers[id.id] { hotkeys.onModule(module) }
            return noErr
        }, 1, &spec, Unmanaged.passUnretained(self).toOpaque(), nil)
    }
}
