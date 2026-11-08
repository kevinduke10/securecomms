import Foundation
import Security

// Device-only items never enter iCloud Keychain or migrate to a replacement device.
enum KeychainStore {
    static let service = "app.cirra.ios"
    static func query(_ name: UnsafePointer<CChar>?) -> [String: Any]? {
        guard let name, let account = String(validatingUTF8: name), !account.isEmpty,
              account.count <= 200, account.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || "-_/".contains($0)) }) else { return nil }
        return [kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: service, kSecAttrAccount as String: account,
                kSecAttrSynchronizable as String: false]
    }
    static let callbacks = KeychainCallbacks(
        get: { name, output, capacity, length in
            guard var query = query(name), let output, let length else { return -1 }
            query[kSecReturnData as String] = true
            query[kSecMatchLimit as String] = kSecMatchLimitOne
            var result: CFTypeRef?
            let status = SecItemCopyMatching(query as CFDictionary, &result)
            if status == errSecItemNotFound { return 1 }
            guard status == errSecSuccess, let data = result as? Data, data.count == 32,
                  capacity >= data.count else { return -1 }
            data.copyBytes(to: output, count: data.count); length.pointee = data.count
            return 0
        },
        set: { name, bytes, length in
            guard var query = query(name), let bytes, length == 32 else { return -1 }
            let data = Data(bytes: bytes, count: length)
            // Create only. A lost/denied existing item must never be overwritten.
            query[kSecValueData as String] = data
            query[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
            return SecItemAdd(query as CFDictionary, nil) == errSecSuccess ? 0 : -1
        },
        delete: { name in
            guard let query = query(name) else { return -1 }
            let status = SecItemDelete(query as CFDictionary)
            return status == errSecSuccess || status == errSecItemNotFound ? 0 : -1
        }
    )
}
