import SwiftUI

@main
struct CirraApp: App {
    var body: some Scene { WindowGroup { CirraView().ignoresSafeArea(.container, edges: .bottom) } }
}
struct CirraView: UIViewControllerRepresentable {
    func makeUIViewController(context: Context) -> CirraViewController { CirraViewController() }
    func updateUIViewController(_ uiViewController: CirraViewController, context: Context) {}
}
