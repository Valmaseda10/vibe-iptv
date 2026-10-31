// Ajustes del sistema que el WebView no expone (F15): «Reducir transparencia» (el cristal
// de la interfaz se vuelve casi opaco) y que la vista web ocupe la pantalla entera.
import UIKit
import WebKit

/// `true` si está activado Ajustes → Accesibilidad → Pantalla → Reducir transparencia.
@_cdecl("vibe_reduce_transparency")
public func vibeReduceTransparency() -> Bool {
    if Thread.isMainThread { return UIAccessibility.isReduceTransparencyEnabled }
    return DispatchQueue.main.sync { UIAccessibility.isReduceTransparencyEnabled }
}

/// La vista web ocupa toda la pantalla y la interfaz se encarga de las zonas de la hora y
/// del indicador de inicio (`env(safe-area-inset-*)` en `globals.css`). Sin esto, algunas
/// veces al abrir la app iOS reservaba además esas zonas: la página quedaba 96 puntos más
/// corta, con la barra de pestañas a media altura y una franja vacía abajo (visto en un
/// iPhone de verdad). Se reintenta un rato porque la vista web aparece después de llamar.
@_cdecl("vibe_webview_full_screen")
public func vibeWebviewFullScreen() {
    DispatchQueue.main.async { applyWebviewFullScreen(attempt: 0) }
}

private func applyWebviewFullScreen(attempt: Int) {
    let webViews = UIApplication.shared.connectedScenes
        .compactMap { $0 as? UIWindowScene }
        .flatMap { $0.windows }
        .flatMap { webViewsInside($0) }
    for webView in webViews {
        let scrollView = webView.scrollView
        scrollView.contentInsetAdjustmentBehavior = .never
        scrollView.contentInset = .zero
        scrollView.scrollIndicatorInsets = .zero
        relayout(webView)
    }
    if webViews.isEmpty, attempt < 40 {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) { applyWebviewFullScreen(attempt: attempt + 1) }
    } else if attempt >= 0 {
        // La primera página puede terminar de cargar después: se vuelve a encajar entonces.
        for delay in [0.8, 2.0] {
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) { applyWebviewFullScreen(attempt: -1) }
        }
    }
}

/// Quitar los márgenes no hace que WebKit recoloque la página: en un iPhone de verdad, al
/// abrir la app la barra de pestañas se quedaba subida con una franja negra debajo, hasta
/// que el giro del reproductor la recolocaba. Un cambio de tamaño de un punto, deshecho al
/// momento, lo fuerza igual que el giro.
private func relayout(_ webView: WKWebView) {
    let frame = webView.frame
    guard frame.height > 1 else { return }
    webView.frame = CGRect(x: frame.minX, y: frame.minY, width: frame.width, height: frame.height - 1)
    webView.layoutIfNeeded()
    DispatchQueue.main.async {
        webView.frame = frame
        webView.layoutIfNeeded()
    }
}

private func webViewsInside(_ view: UIView) -> [WKWebView] {
    if let webView = view as? WKWebView { return [webView] }
    return view.subviews.flatMap { webViewsInside($0) }
}
