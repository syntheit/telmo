import Foundation

/// Forwards `helper <request>` to the optional root helper (telmo-helper).
/// The helper only answers processes signed by this app's team, which is why
/// the modules go through the host instead of calling it directly.
enum Helper {
    private static let socketPath = "/var/run/telmo-helper.sock"

    static func isHelperCommand(_ text: String) -> Bool {
        text.hasPrefix("helper ")
    }

    /// Runs on the connection's worker thread; blocking I/O is fine here.
    static func forward(_ text: String) -> String {
        let request = String(text.dropFirst("helper ".count)) + "\n"
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { return "error The helper isn't reachable." }
        defer { close(fd) }
        var timeout = timeval(tv_sec: 3, tv_usec: 0)
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
        setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))

        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: socketPath.utf8) }
        let connected = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 else { return "error The helper isn't running." }

        let bytes = Array(request.utf8)
        guard write(fd, bytes, bytes.count) == bytes.count else { return "error The helper didn't accept the request." }

        // The helper answers once and closes; read until then.
        var reply = [UInt8]()
        var chunk = [UInt8](repeating: 0, count: 4096)
        while true {
            let count = read(fd, &chunk, chunk.count)
            if count <= 0 { break }
            reply.append(contentsOf: chunk[..<count])
        }
        let text = String(decoding: reply, as: UTF8.self).trimmingCharacters(in: .newlines)
        return text.isEmpty ? "error The helper gave no answer." : text
    }
}
