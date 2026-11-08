import XCTest
import UIKit
import WebKit
@testable import Cirra
final class BoundaryTests: XCTestCase {
    func testOnlyBundledMainOriginIsAllowed() {
        XCTAssertTrue(AppBoundary.isBundled(URL(string:"cirra-app://bundle/index.html?chat=fixture")))
        for value in ["https://relay.example","file:///private/key","cirra-app://attacker/","cirra-app://user@bundle/","cirra-app://bundle:443/"] {
            XCTAssertFalse(AppBoundary.isBundled(URL(string:value)),value)
        }
    }
    func testExportNamesCannotBecomePaths() {
        for name in ["", " ", ".", "..", "../key", "folder/file", "folder\\file", "disk:file", "bad\nname", String(repeating:"x",count:181)] { XCTAssertFalse(AppBoundary.validFilename(name),name) }
        for name in ["photo.png","cirra-history.crkbackup","report final.pdf","📎.txt"] { XCTAssertTrue(AppBoundary.validFilename(name),name) }
    }
    @MainActor
    func testBundledReactLoadsAndNativeStatusStaysIsolated() async throws {
        func findWebView(_ view:UIView)->WKWebView? {
            if let web=view as? WKWebView{return web}
            for child in view.subviews {if let web=findWebView(child){return web}}
            return nil
        }
        var web:WKWebView?
        for _ in 0..<100 {
            let windows=UIApplication.shared.connectedScenes.compactMap{$0 as? UIWindowScene}.flatMap{$0.windows}
            web=windows.compactMap{findWebView($0)}.first
            if let web, (try? await web.evaluateJavaScript("document.querySelector('h1')?.textContent || ''")) as? String == "Sign in to Cirra" {break}
            try await Task.sleep(nanoseconds:100_000_000)
        }
        let view=try XCTUnwrap(web)
        let heading=try await view.evaluateJavaScript("document.querySelector('h1')?.textContent || ''")
        XCTAssertEqual(heading as? String,"Sign in to Cirra")
        var body=""
        for _ in 0..<50 {
            body=(try await view.evaluateJavaScript("document.body.innerText || ''")) as? String ?? ""
            if body.contains("Business relay URL"){break}
            try await Task.sleep(nanoseconds:100_000_000)
        }
        XCTAssertTrue(body.contains("Business relay URL"),body)
        let response=try await view.callAsyncJavaScript("return await window.__CIRRA_IOS__.invoke('client_request',{request:{method:'GET',path:'/desktop/status',body:null}})",arguments:[:],in:nil,contentWorld:.page)
        let result=try XCTUnwrap(response as? [String:Any]);XCTAssertEqual(result["status"] as? Int,200)
        let state=try XCTUnwrap(result["body"] as? [String:Any]);XCTAssertEqual(state["configured"] as? Bool,false);XCTAssertEqual(state["approved"] as? Bool,false)
        let blocked=try await view.callAsyncJavaScript("return await window.__CIRRA_IOS__.invoke('client_request',{request:{method:'POST',path:'/desktop/auth/start',body:{}}}).then(()=>false,()=>true)",arguments:[:],in:nil,contentWorld:.page)
        XCTAssertEqual(blocked as? Bool,true)
        let history=try await view.evaluateJavaScript("(()=>{const before=location.href;history.replaceState(null,'',new URL('?chat=fixture',location.href));history.replaceState(null,'',before);return 'ok'})()")
        XCTAssertEqual(history as? String,"ok")
        NotificationCenter.default.post(name:UIApplication.willResignActiveNotification,object:nil)
        try await Task.sleep(nanoseconds:100_000_000)
        let inactive=try await view.evaluateJavaScript("window.__CIRRA_IOS_ACTIVE__");XCTAssertEqual(inactive as? Bool,false)
        NotificationCenter.default.post(name:UIApplication.didBecomeActiveNotification,object:nil)
        try await Task.sleep(nanoseconds:100_000_000)
        let active=try await view.evaluateJavaScript("window.__CIRRA_IOS_ACTIVE__");XCTAssertEqual(active as? Bool,true)
    }

}
