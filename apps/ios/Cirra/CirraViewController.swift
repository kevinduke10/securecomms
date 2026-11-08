import UIKit
@preconcurrency import WebKit
import AuthenticationServices

private final class MessageRelay: NSObject, WKScriptMessageHandler {
    weak var owner: CirraViewController?
    init(_ owner: CirraViewController) { self.owner = owner }
    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) { owner?.userContentController(controller,didReceive:message) }
}
final class CirraViewController: UIViewController, WKScriptMessageHandler, WKNavigationDelegate,
    ASWebAuthenticationPresentationContextProviding, UIDocumentPickerDelegate {
    private let engine = DeviceEngine()
    private var webView: WKWebView!
    private var login: ASWebAuthenticationSession?
    private var exportCompletion: ((Bool) -> Void)?
    private var exportFile: URL?
    private var pending = Set<Int>()
    private var privacyView: UIView?
    private var foreground = false
    private var authOccluded = false
    private var exportOccluded = false
    private var loginGeneration = UUID()
    private static let bridge = """
    (()=>{
      if(location.protocol!=='cirra-app:'||location.host!=='bundle'||window!==window.top)return;
      let sequence=0;const pending=new Map();let nativeActive=false;
      Object.defineProperty(window,'__CIRRA_IOS_ACTIVE__',{get:()=>nativeActive});
      Object.defineProperty(window,'__cirraSetNativeActive',{value:active=>{const next=Boolean(active);if(nativeActive===next)return;nativeActive=next;window.dispatchEvent(new Event('cirra-native-visibility'));if(!nativeActive)window.dispatchEvent(new Event('blur'));}});
      Object.defineProperty(window,'__cirraResolve',{value:(id,ok,value)=>{const call=pending.get(id);if(!call)return;pending.delete(id);ok?call.resolve(value):call.reject(new Error(String(value)));}});
      Object.defineProperty(window,'__CIRRA_IOS__',{value:Object.freeze({invoke:(command,args={})=>new Promise((resolve,reject)=>{const id=++sequence;pending.set(id,{resolve,reject});window.webkit.messageHandlers.cirra.postMessage({id,command,args});})})});
      if(!crypto.randomUUID)Object.defineProperty(crypto,'randomUUID',{value:()=>{const b=new Uint8Array(16);crypto.getRandomValues(b);b[6]=(b[6]&15)|64;b[8]=(b[8]&63)|128;const h=Array.from(b,v=>v.toString(16).padStart(2,'0')).join('');return `${h.slice(0,8)}-${h.slice(8,12)}-${h.slice(12,16)}-${h.slice(16,20)}-${h.slice(20)}`;}});
    })();
    """
    override func viewDidLoad() {
        super.viewDidLoad(); view.backgroundColor = .systemBackground
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        config.setURLSchemeHandler(BundledAssets(root: Bundle.main.resourceURL!.appendingPathComponent("Web")), forURLScheme: AppBoundary.scheme)
        config.userContentController.addUserScript(WKUserScript(source: Self.bridge, injectionTime: .atDocumentStart, forMainFrameOnly: true))
        config.userContentController.add(MessageRelay(self), name: "cirra")
        webView = WKWebView(frame: .zero, configuration: config); webView.navigationDelegate = self
        webView.translatesAutoresizingMaskIntoConstraints = false
        webView.isOpaque = false; webView.backgroundColor = .systemBackground
        webView.scrollView.contentInsetAdjustmentBehavior = .never
        view.addSubview(webView)
        NSLayoutConstraint.activate([webView.leadingAnchor.constraint(equalTo: view.leadingAnchor), webView.trailingAnchor.constraint(equalTo: view.trailingAnchor), webView.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor), webView.bottomAnchor.constraint(equalTo: view.bottomAnchor)])
        NotificationCenter.default.addObserver(self, selector: #selector(hideContent), name: UIApplication.willResignActiveNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(showContent), name: UIApplication.didBecomeActiveNotification, object: nil)
        engine.open { [weak self] result in
            switch result {
            case .success: self?.webView.load(URLRequest(url: URL(string: "cirra-app://bundle/")!))
            case .failure(let error): self?.showStartupError(error.localizedDescription)
            }
        }
    }
    private func showStartupError(_ message: String) {
        let label = UILabel(); label.numberOfLines = 0; label.text = "Cirra\n\n\(message)"; label.textAlignment = .center
        label.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(label)
        NSLayoutConstraint.activate([label.centerYAnchor.constraint(equalTo: view.centerYAnchor),label.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 24),label.trailingAnchor.constraint(equalTo: view.trailingAnchor,constant: -24)])
    }
    private func hasNativeOverlay(_ controller:UIViewController?) -> Bool {
        guard let controller else {return false}
        return controller.presentedViewController != nil || controller.children.contains(where:hasNativeOverlay)
    }
    private var canAnnounceActivity:Bool {
        foreground && UIApplication.shared.applicationState == .active && !authOccluded && !exportOccluded && !hasNativeOverlay(view.window?.rootViewController)
    }
    private func syncActivity() {
        let active=canAnnounceActivity
        webView.callAsyncJavaScript("window.__cirraSetNativeActive?.(active);return true;",arguments:["active":active],in:nil,in:.page,completionHandler:{_ in})
    }
    @objc private func hideContent() {
        foreground=false;syncActivity()
        guard privacyView == nil else { return }
        let host=view.window ?? view!
        let cover=UIView(frame:host.bounds);cover.backgroundColor = .systemBackground
        cover.autoresizingMask=[.flexibleWidth,.flexibleHeight];host.addSubview(cover);privacyView=cover
    }
    @objc private func showContent() {
        foreground=UIApplication.shared.applicationState == .active;syncActivity()
        privacyView?.removeFromSuperview();privacyView=nil
    }
    func webView(_ webView:WKWebView,didFinish navigation:WKNavigation!) {
        foreground=UIApplication.shared.applicationState == .active;syncActivity()
    }
    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        decisionHandler(AppBoundary.isBundled(navigationAction.request.url) && navigationAction.targetFrame?.isMainFrame == true ? .allow : .cancel)
    }
    func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
        guard message.name == "cirra", message.frameInfo.isMainFrame,
              AppBoundary.isBundled(message.frameInfo.request.url), AppBoundary.isBundled(webView.url),
              message.frameInfo.securityOrigin.protocol == AppBoundary.scheme,
              message.frameInfo.securityOrigin.host == AppBoundary.host,
              let body = message.body as? [String: Any], let id = body["id"] as? Int,
              id > 0, pending.count < 64, !pending.contains(id),
              let command = body["command"] as? String, let args = body["args"] as? [String: Any] else { return }
        pending.insert(id)
        switch command {
        case "client_request":
            guard let request = args["request"] as? [String: Any], let method = request["method"] as? String,
                  let path = request["path"] as? String, ["GET","POST"].contains(method), !path.hasPrefix("/desktop/auth/start") else { reject(id, "Unsupported client request."); return }
            // WebKit's built-in attachment chooser can be presented by an ancestor
            // controller. Recheck native occlusion before a read/typing claim,
            // independently of the renderer's most recent visibility signal.
            syncActivity()
            let input=request["body"] as? [String:Any]
            if (path.hasSuffix("/read") || (path.hasSuffix("/typing") && input?["active"] as? Bool == true)) && !canAnnounceActivity {
                reject(id,"The conversation is not currently visible.");return
            }
            if path == "/local-auth/logout" { loginGeneration=UUID();login?.cancel();login=nil;authOccluded=false;syncActivity();engine.cancelLogin() }
            engine.request(request) { [weak self] result in self?.complete(id, result) }
        case "open_login":
            guard args.isEmpty else { reject(id,"Login does not accept a URL."); return }
            openLogin(id)
        case "save_download":
            guard let filename = args["filename"] as? String, let data = args["data"] as? String else { reject(id,"Invalid file export."); return }
            export(id, filename: filename, encoded: data)
        case "desktop_info": resolve(id, ["platform":"ios"])
        default: reject(id,"Unsupported native command.")
        }
    }
    private func complete(_ id: Int, _ result: Result<Any, Error>) {
        switch result { case .success(let value): resolve(id,value); case .failure(let error): reject(id,error.localizedDescription) }
    }
    private func reply(_ id: Int, success: Bool, value: Any) {
        guard pending.remove(id) != nil, AppBoundary.isBundled(webView.url) else { return }
        webView.callAsyncJavaScript("window.__cirraResolve(id,success,value)", arguments: ["id":id,"success":success,"value":value], in: nil, in: .page, completionHandler: { _ in })
    }
    private func resolve(_ id: Int, _ value: Any) { reply(id, success:true,value:value) }
    private func reject(_ id: Int, _ error: String) { reply(id,success:false,value:error) }
    private func openLogin(_ id: Int) {
        let generation=UUID();loginGeneration=generation
        login?.cancel();login=nil;authOccluded=false;syncActivity();engine.cancelLogin()
        engine.beginLogin { [weak self] result in
            guard let self else { return }
            guard self.loginGeneration==generation else {self.reject(id,"Sign-in was replaced.");return}
            do {
                let value = try result.get()
                guard let data = value as? [String:Any], let text = data["authorize_url"] as? String, let url = URL(string:text),
                      url.scheme == "https", url.path == "/oauth2/authorize", url.user == nil, url.password == nil else { throw EngineFailure(message:"Invalid account authorization URL.") }
                let session = ASWebAuthenticationSession(url:url, callbackURLScheme:"app.cirra.ios") { [weak self] callback, error in
                    DispatchQueue.main.async {
                        guard let self,self.loginGeneration==generation else {return}
                        self.login=nil;self.authOccluded=false;self.syncActivity()
                        guard error == nil, let callback else { self.engine.cancelLogin(); return }
                        self.engine.completeLogin(callback) { result in
                            if case .failure(let error) = result { self.showAuthError(error.localizedDescription) }
                        }
                    }
                }
                session.presentationContextProvider = self; session.prefersEphemeralWebBrowserSession = true
                self.login=session;self.authOccluded=true;self.syncActivity()
                guard session.start() else {self.login=nil;self.authOccluded=false;self.syncActivity();self.engine.cancelLogin();throw EngineFailure(message:"Unable to open account sign-in.")}
                self.resolve(id,NSNull())
            } catch {self.authOccluded=false;self.syncActivity();self.engine.cancelLogin();self.reject(id,error.localizedDescription)}
        }
    }
    private func showAuthError(_ message:String) {
        let alert = UIAlertController(title:"Sign-in failed",message:message,preferredStyle:.alert)
        alert.addAction(UIAlertAction(title:"Close",style:.default)); present(alert,animated:true)
    }
    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor { view.window! }
    private func export(_ id:Int,filename:String,encoded:String) {
        guard exportCompletion == nil, AppBoundary.validFilename(filename), encoded.utf8.count <= 40*1024*1024,
              let data=Data(base64Encoded:encoded), data.count <= 27*1024*1024 else { reject(id,"Invalid or oversized file export."); return }
        do {
            let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString,isDirectory:true)
            try FileManager.default.createDirectory(at: directory,withIntermediateDirectories:true,attributes:[.protectionKey:FileProtectionType.complete])
            let file=directory.appendingPathComponent(filename)
            try data.write(to:file,options:[.atomic,.completeFileProtection]); exportFile=file
            exportCompletion = { [weak self] saved in self?.resolve(id,saved); self?.clearExport() }
            let picker=UIDocumentPickerViewController(forExporting:[file],asCopy:true);picker.delegate=self
            exportOccluded=true;syncActivity();present(picker,animated:true)
        } catch { clearExport(); reject(id,"Unable to prepare file export.") }
    }
    private func clearExport() { if let file=exportFile { try? FileManager.default.removeItem(at:file.deletingLastPathComponent()) };exportFile=nil;exportCompletion=nil;exportOccluded=false;syncActivity() }
    func documentPickerWasCancelled(_ controller:UIDocumentPickerViewController) {controller.dismiss(animated:true){[weak self] in self?.exportCompletion?(false)}}
    func documentPicker(_ controller:UIDocumentPickerViewController,didPickDocumentsAt urls:[URL]) {controller.dismiss(animated:true){[weak self] in self?.exportCompletion?(!urls.isEmpty)}}
    deinit { NotificationCenter.default.removeObserver(self) }
}
