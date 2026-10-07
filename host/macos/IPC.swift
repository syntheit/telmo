import Foundation

enum ModuleLookup {
    /// launchd's PATH is minimal, so add the login PATH and the usual Nix profile directories.
    static let searchPath: [String] = {
        var dirs: [String] = []
        if let dir = ProcessInfo.processInfo.environment["TELMO_BIN_DIR"] { dirs.append(dir) }
        dirs += pathHelperDirs()
        dirs += [NSHomeDirectory() + "/.nix-profile/bin", "/etc/profiles/per-user/\(NSUserName())/bin"]
        dirs += (ProcessInfo.processInfo.environment["PATH"] ?? "").split(separator: ":").map(String.init)
        var seen = Set<String>()
        return dirs.filter { !$0.isEmpty && seen.insert($0).inserted }
    }()

    static func find(_ module: String) -> String? {
        guard module.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "-" }) else { return nil }
        let name = "telmo-\(module)"
        let own = Bundle.main.bundleURL.appendingPathComponent("Contents")
        let dirs = [own.appendingPathComponent("MacOS").path, own.appendingPathComponent("Helpers").path] + searchPath
        return dirs.map { "\($0)/\(name)" }.first { FileManager.default.isExecutableFile(atPath: $0) }
    }

    /// A program from a popup command: a path as given, or a name looked up like the modules.
    static func findExecutable(_ program: String) -> String? {
        if program.contains("/") { return FileManager.default.isExecutableFile(atPath: program) ? program : nil }
        return searchPath.map { "\($0)/\(program)" }.first { FileManager.default.isExecutableFile(atPath: $0) }
    }

    private static func pathHelperDirs() -> [String] {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/libexec/path_helper")
        task.arguments = ["-s"]
        let pipe = Pipe()
        task.standardOutput = pipe
        guard (try? task.run()) != nil else { return [] }
        let output = String(decoding: pipe.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        // PATH="a:b:c"; export PATH;
        guard let start = output.range(of: "PATH=\""), let end = output[start.upperBound...].firstIndex(of: "\"") else { return [] }
        return output[start.upperBound..<end].split(separator: ":").map(String.init)
    }
}

/// Line protocol over a unix socket: one request line, then a reply that ends when the host closes the connection. Connections are handled on a background queue
/// with a read timeout; commands run on the main thread asynchronously.
final class IPCServer {
    static let socketPath = ProcessInfo.processInfo.environment["TELMO_SOCKET"]
        ?? NSHomeDirectory() + "/Library/Application Support/Telmo/host.sock"

    private let handler: (String) -> String
    private var listenFD: Int32 = -1
    private let queue = DispatchQueue(label: "telmo.ipc", attributes: .concurrent)

    init(handler: @escaping (String) -> String) { self.handler = handler }

    private static func address() -> sockaddr_un {
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(socketPath.utf8CString)
        withUnsafeMutableBytes(of: &addr.sun_path) { dst in
            for (i, b) in bytes.prefix(dst.count - 1).enumerated() { dst[i] = UInt8(bitPattern: b) }
        }
        return addr
    }

    private static func connectToSocket() -> Int32? {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { return nil }
        var addr = address()
        let ok = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        if ok == 0 { return fd }
        close(fd)
        return nil
    }

    static func isRunning() -> Bool {
        guard let fd = connectToSocket() else { return false }
        close(fd)
        return true
    }

    func start() throws {
        let dir = (Self.socketPath as NSString).deletingLastPathComponent
        try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        // Passwords travel over this socket. An override may point into a shared directory such as /tmp, which must stay as it is.
        if ProcessInfo.processInfo.environment["TELMO_SOCKET"] == nil { chmod(dir, 0o700) }
        unlink(Self.socketPath) // stale: isRunning() already failed to connect

        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw POSIXError(.EIO) }
        var addr = Self.address()
        let bound = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        guard bound == 0, listen(fd, 8) == 0 else {
            let err = errno
            close(fd)
            throw POSIXError(POSIXErrorCode(rawValue: err) ?? .EIO)
        }
        chmod(Self.socketPath, 0o600)
        fcntl(fd, F_SETFD, FD_CLOEXEC)
        listenFD = fd
        Thread.detachNewThread { [self] in acceptLoop() }
    }

    private func acceptLoop() {
        while true {
            let client = accept(listenFD, nil, nil)
            if client < 0 {
                if errno == EINTR || errno == ECONNABORTED { continue }
                return
            }
            fcntl(client, F_SETFD, FD_CLOEXEC) // the popup's child must not inherit the connection
            // A client that went away (the popup's child is killed while we answer it) must not kill the host with SIGPIPE.
            var on: Int32 = 1
            setsockopt(client, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
            queue.async { [self] in serve(client) }
        }
    }

    private func serve(_ fd: Int32) {
        defer { close(fd) }
        var timeout = timeval(tv_sec: 1, tv_usec: 0)
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
        setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
        var line = [UInt8]()
        var byte: UInt8 = 0
        while line.count < 1024, read(fd, &byte, 1) == 1, byte != 10 { line.append(byte) }
        // Only line ends are trimmed: SSIDs and passwords may start or end with spaces.
        let text = String(decoding: line, as: UTF8.self).trimmingCharacters(in: .newlines)
        guard !text.isEmpty else { return }
        if Audio.isAudioCommand(text) { return Audio.stream(to: fd) } // streams until the client leaves
        if Wifi.isWifiCommand(text) { return reply(Wifi.handle(text), to: fd) } // slow: stays off the main thread
        // Main runs the command asynchronously; this background thread waits (bounded) for the reply.
        let done = DispatchSemaphore(value: 0)
        let lock = NSLock()
        var result: String?
        DispatchQueue.main.async { [handler] in
            let value = handler(text)
            lock.lock()
            result = value
            lock.unlock()
            done.signal()
        }
        _ = done.wait(timeout: .now() + 1)
        lock.lock()
        let reply = result ?? "error host busy"
        lock.unlock()
        self.reply(reply, to: fd)
    }

    private func reply(_ text: String, to fd: Int32) {
        var timeout = timeval(tv_sec: 10, tv_usec: 0)
        setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
        var bytes = Array((text + "\n").utf8)[...]
        while !bytes.isEmpty {
            let n = bytes.withUnsafeBytes { write(fd, $0.baseAddress, $0.count) }
            if n < 0 && errno == EINTR { continue }
            if n <= 0 { break }
            bytes = bytes.dropFirst(n)
        }
    }
}
