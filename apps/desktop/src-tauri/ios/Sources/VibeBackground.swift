// Comprobación de episodios nuevos en segundo plano (F17, parte 4). iOS decide cuándo
// despierta la app (se le pide «no antes de 6 horas»; con la app firmada gratis, o si casi
// no se abre, puede tardar mucho más o no hacerlo). Al despertarla, el núcleo pide al
// proveedor las series seguidas (`catalog::new_episodes`) y avisa de lo nuevo.
//
// Lo que entra: `vibe_background_register()`, desde Rust al arrancar (antes de que acabe
// el arranque: iOS no deja registrar la tarea después).
// Lo que sale: `vibe_background_refresh()`, en Rust: hace la comprobación (≤ 25 s).

import BackgroundTasks
import UIKit

private let refreshTaskId = "dev.vibeiptv.desktop.refresh"
private let earliestDelay: TimeInterval = 6 * 3600

@_silgen_name("vibe_background_refresh")
private func vibeBackgroundRefreshInRust() -> Bool

private func scheduleNextRefresh() {
    let request = BGAppRefreshTaskRequest(identifier: refreshTaskId)
    request.earliestBeginDate = Date(timeIntervalSinceNow: earliestDelay)
    do {
        try BGTaskScheduler.shared.submit(request)
    } catch {
        // En el simulador no hay tareas en segundo plano: no pasa nada.
        NSLog("vibe: no se pudo programar la comprobación en segundo plano: \(error)")
    }
}

private var registered = false

@_cdecl("vibe_background_register")
public func vibeBackgroundRegister() {
    guard !registered else { return }
    registered = true
    let ok = BGTaskScheduler.shared.register(forTaskWithIdentifier: refreshTaskId, using: nil) { task in
        guard let task = task as? BGAppRefreshTask else { return }
        // La siguiente, antes de nada: si esta se corta, queda la otra.
        scheduleNextRefresh()
        let work = DispatchWorkItem {
            let ok = vibeBackgroundRefreshInRust()
            task.setTaskCompleted(success: ok)
        }
        task.expirationHandler = { work.cancel() }
        DispatchQueue.global(qos: .utility).async(execute: work)
    }
    NSLog("vibe: comprobación en segundo plano registrada: \(ok)")
    scheduleNextRefresh()
    // Cada vez que la app pasa a segundo plano, se vuelve a pedir (iOS solo guarda una).
    NotificationCenter.default.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { _ in
        scheduleNextRefresh()
    }
}
