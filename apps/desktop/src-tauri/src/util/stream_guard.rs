//! Semáforo global de una única conexión de stream activa (regla R1 de CLAUDE.md).
//!
//! F2 implementa el contrato completo (adquirir/liberar/consultar, liberación garantizada
//! incluso en pánico, y que una nueva sesión invalida la anterior) sin un proceso real
//! que cerrar todavía: `media/ffmpeg.rs` y `media/mpv.rs` siguen vacíos hasta F6. Cuando
//! existan, `acquire` esperará de verdad a que el proceso anterior muera antes de
//! devolver el permiso siguiente, tal como describe `docs/ARCHITECTURE.md` — por eso ya
//! es `async` ahora, aunque hoy no haya nada que esperar.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Default)]
pub struct StreamGuard {
    generation: AtomicU64,
    active: AtomicBool,
}

impl StreamGuard {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Marca una sesión de stream como activa. Invalida cualquier sesión anterior: solo
    /// puede haber una a la vez.
    pub async fn acquire(self: &Arc<Self>) -> StreamPermit {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.active.store(true, Ordering::SeqCst);
        StreamPermit { guard: Arc::clone(self), generation }
    }

    /// El sincronizador de catálogo la consulta antes de cada lote (R1): se pausa
    /// mientras haya reproducción activa, nunca cancela por eso.
    pub fn is_busy(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
}

/// Permiso de reproducción. Al soltarse (incluido un pánico durante la sesión) libera el
/// guard, salvo que ya se haya adquirido una sesión más nueva mientras tanto.
pub struct StreamPermit {
    guard: Arc<StreamGuard>,
    generation: u64,
}

impl Drop for StreamPermit {
    fn drop(&mut self) {
        if self.guard.generation.load(Ordering::SeqCst) == self.generation {
            self.guard.active.store(false, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn is_busy_reflects_acquire_and_drop() {
        let guard = StreamGuard::new();
        assert!(!guard.is_busy());

        let permit = guard.acquire().await;
        assert!(guard.is_busy());

        drop(permit);
        assert!(!guard.is_busy());
    }

    #[tokio::test]
    async fn acquiring_again_invalidates_the_previous_permits_release() {
        let guard = StreamGuard::new();
        let first = guard.acquire().await;
        let second = guard.acquire().await;
        assert!(guard.is_busy());

        // Soltar el permiso viejo llega tarde: no debe pisar la sesión vigente.
        drop(first);
        assert!(guard.is_busy());

        drop(second);
        assert!(!guard.is_busy());
    }

    #[tokio::test]
    async fn releases_even_if_a_panic_happens_while_holding_the_permit() {
        let guard = StreamGuard::new();
        // Adquirir el permiso es la única parte async; el pánico en sí se dispara en un
        // cierre síncrono normal, que es donde `catch_unwind` puede envolverlo.
        let permit = guard.acquire().await;

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _permit = permit;
            panic!("simulando un fallo durante la reproducción");
        }));
        assert!(result.is_err());

        assert!(!guard.is_busy());
    }
}
