// Tipo de conexión del iPhone/iPad (F15): el tráiler de fondo de las fichas solo se
// reproduce con Wi‑Fi, para no gastar datos móviles. El WebView no sabe distinguirlo.
import Foundation
import Network

private let pathMonitor: NWPathMonitor = {
    let monitor = NWPathMonitor()
    monitor.start(queue: DispatchQueue(label: "vibe.network"))
    return monitor
}()

/// Arranca la vigilancia de la red; la primera lectura tarda un instante en llegar.
@_cdecl("vibe_network_start")
public func vibeNetworkStart() {
    _ = pathMonitor
}

/// `true` si ahora mismo la conexión cuesta datos (móvil, punto de acceso) o no hay red.
@_cdecl("vibe_network_is_metered")
public func vibeNetworkIsMetered() -> Bool {
    let path = pathMonitor.currentPath
    return path.status != .satisfied || path.isExpensive || path.isConstrained || path.usesInterfaceType(.cellular)
}
