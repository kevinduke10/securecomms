import Foundation

struct EngineFailure: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}
// All Rust handle access, SQLite operations and Keychain callbacks run on this
// queue. The webview and authentication presentation remain on the main thread.
final class DeviceEngine {
    private let queue = DispatchQueue(label: "app.cirra.ios.device", qos: .userInitiated)
    private var handle: OpaquePointer?
    func open(completion: @escaping (Result<Void, Error>) -> Void) {
        queue.async {
            let result = Result<Void, Error> {
                guard self.handle == nil else { return }
                guard cirra_ios_abi_version() == 1 else { throw EngineFailure(message: "The app and Rust client versions do not match.") }
                let base = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
                let storeName = ProcessInfo.processInfo.environment["CIRRA_IOS_TESTING"] == "1" ? "CirraFixture-" + UUID().uuidString : "CirraDevice"
                var directory = base.appendingPathComponent(storeName, isDirectory: true)
                try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true,
                    attributes: [.protectionKey: FileProtectionType.complete])
                var values = URLResourceValues(); values.isExcludedFromBackup = true
                try directory.setResourceValues(values)
                var error: UnsafeMutablePointer<CChar>?
                self.handle = directory.path.withCString { cirra_ios_open($0, KeychainStore.callbacks, &error) }
                if self.handle == nil {
                    if let error { let failure = try self.decode(error); throw EngineFailure(message: failure["error"] as? String ?? "Unable to open device store.") }
                    throw EngineFailure(message: "Unable to open device store.")
                }
            }
            DispatchQueue.main.async { completion(result) }
        }
    }
    private func decode(_ pointer: UnsafeMutablePointer<CChar>?) throws -> [String: Any] {
        guard let pointer else { throw EngineFailure(message: "The native client returned no response.") }
        defer { cirra_ios_free(pointer) }
        guard let data = String(validatingUTF8: pointer)?.data(using: .utf8),
              let result = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw EngineFailure(message: "The native client returned an invalid response.")
        }
        return result
    }
    private func perform(_ operation: @escaping (OpaquePointer) -> UnsafeMutablePointer<CChar>?, completion: @escaping (Result<Any, Error>) -> Void) {
        queue.async {
            let result = Result<Any, Error> {
                guard let handle = self.handle else { throw EngineFailure(message: "The device store is not open.") }
                let result = try self.decode(operation(handle))
                guard result["ok"] as? Bool == true else { throw EngineFailure(message: result["error"] as? String ?? "Device operation failed.") }
                return result["value"] ?? NSNull()
            }
            DispatchQueue.main.async { completion(result) }
        }
    }
    func request(_ request: [String: Any], completion: @escaping (Result<Any, Error>) -> Void) {
        do {
            let data = try JSONSerialization.data(withJSONObject: request)
            guard data.count <= 40 * 1024 * 1024, let string = String(data: data, encoding: .utf8) else { throw EngineFailure(message: "Client request is too large.") }
            perform({ handle in string.withCString { cirra_ios_request(handle, $0) } }, completion: completion)
        } catch { completion(.failure(error)) }
    }
    func beginLogin(completion: @escaping (Result<Any, Error>) -> Void) { perform({ cirra_ios_begin_login($0) }, completion: completion) }
    func completeLogin(_ callback: URL, completion: @escaping (Result<Any, Error>) -> Void) {
        perform({ handle in callback.absoluteString.withCString { cirra_ios_complete_login(handle, $0) } }, completion: completion)
    }
    func cancelLogin() { queue.async { if let handle = self.handle { cirra_ios_cancel_login(handle) } } }
    deinit {
        // A captured strong self keeps requests alive until their queued work ends.
        if let handle { queue.async { cirra_ios_close(handle) } }
    }
}
