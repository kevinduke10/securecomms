import Foundation
import WebKit

enum AppBoundary {
    static let scheme = "cirra-app"
    static let host = "bundle"
    static func isBundled(_ url: URL?) -> Bool {
        guard let url else { return false }
        return url.scheme == scheme && url.host == host && url.user == nil && url.password == nil && url.port == nil
    }
    static func validFilename(_ name: String) -> Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && name.utf8.count <= 180 && name != "." && name != ".." &&
        !name.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) || "/\\:".unicodeScalars.contains($0) })
    }
}
final class BundledAssets: NSObject, WKURLSchemeHandler {
    private let root: URL
    init(root: URL) { self.root = root.standardizedFileURL }
    func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
        do {
            guard let url = task.request.url, AppBoundary.isBundled(url), task.request.httpMethod == "GET" else { throw URLError(.unsupportedURL) }
            let path = url.path.isEmpty || url.path == "/" ? "index.html" : String(url.path.dropFirst())
            let file = root.appendingPathComponent(path).standardizedFileURL
            guard file.path.hasPrefix(root.path + "/"), !path.contains(".."), FileManager.default.fileExists(atPath: file.path) else { throw URLError(.fileDoesNotExist) }
            let mime: String
            switch file.pathExtension.lowercased() {
            case "html": mime = "text/html; charset=utf-8"
            case "js": mime = "text/javascript; charset=utf-8"
            case "css": mime = "text/css; charset=utf-8"
            case "svg": mime = "image/svg+xml"
            case "png": mime = "image/png"
            case "json": mime = "application/json"
            default: mime = "application/octet-stream"
            }
            let data = try Data(contentsOf: file, options: .mappedIfSafe)
            let response = HTTPURLResponse(url: url, statusCode: 200, httpVersion: "HTTP/1.1", headerFields: [
                "Content-Type": mime, "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff",
                "Content-Security-Policy": "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; media-src 'self' blob:; connect-src 'self'; object-src 'none'; frame-src 'none'; base-uri 'none'; form-action 'none'"
            ])!
            task.didReceive(response); task.didReceive(data); task.didFinish()
        } catch { task.didFailWithError(error) }
    }
    func webView(_ webView: WKWebView, stop task: WKURLSchemeTask) {}
}
