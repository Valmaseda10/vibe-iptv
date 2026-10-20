// Ajustes de accesibilidad del sistema que el WebView no expone (F15): con «Reducir
// transparencia», el cristal de la interfaz se vuelve casi opaco.
import UIKit

/// `true` si está activado Ajustes → Accesibilidad → Pantalla → Reducir transparencia.
@_cdecl("vibe_reduce_transparency")
public func vibeReduceTransparency() -> Bool {
    if Thread.isMainThread { return UIAccessibility.isReduceTransparencyEnabled }
    return DispatchQueue.main.sync { UIAccessibility.isReduceTransparencyEnabled }
}
