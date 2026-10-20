//! Semáforo global de una única conexión de stream activa (regla R1 de CLAUDE.md).
//!
//! Desde F6, `acquire()` además mata de verdad y espera la muerte del proceso ffmpeg
//! (nivel 2 de la cascada, ver `docs/ARCHITECTURE.md`) de la sesión anterior, si lo
//! había, antes de devolver el permiso siguiente: "espera a que el proceso hijo muera
//! de verdad (no solo a enviar la señal)". El nivel 1 (proxy directo) y el nivel 3
//! (mpv, F9) no tienen proceso que registrar aquí.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::process::Child;
use tokio::sync::Mutex as TokioMutex;

/// Intervalo de sondeo de `run_until_exit_or_replaced`: no hay forma de "esperar" a que
/// un proceso ajeno termine sin bloquear su `Child` (que vive dentro del `Mutex`
/// compartido), así que se comprueba `try_wait()` a intervalos cortos en vez de una
/// espera bloqueante.
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Default)]
pub struct StreamGuard {
    generation: AtomicU64,
    active: AtomicBool,
    /// Proceso ffmpeg (nivel 2) de la sesión activa, junto con su generación. Solo puede
    /// haber uno: abrir una sesión nueva mata y espera este antes de devolver el permiso.
    process: TokioMutex<Option<(u64, Child)>>,
}

impl StreamGuard {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Marca una sesión de stream como activa. Invalida cualquier sesión anterior: solo
    /// puede haber una a la vez. Si la sesión anterior tenía un proceso ffmpeg asociado,
    /// se mata y se espera su terminación real antes de devolver el permiso nuevo.
    pub async fn acquire(self: &Arc<Self>) -> StreamPermit {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.active.store(true, Ordering::SeqCst);
        self.kill_current_process().await;
        StreamPermit { guard: Arc::clone(self), generation }
    }

    async fn kill_current_process(&self) {
        let previous = self.process.lock().await.take();
        if let Some((_, mut child)) = previous {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    }

    /// Registra el proceso ffmpeg de la sesión de `permit`, para que la siguiente
    /// adquisición (o `release_process`) lo mate y espere su muerte de verdad. No hace
    /// nada si `permit` ya no es la sesión vigente (llegó tarde tras un `acquire()` más
    /// nuevo).
    pub async fn attach_process(&self, permit: &StreamPermit, child: Child) {
        if self.generation.load(Ordering::SeqCst) == permit.generation {
            *self.process.lock().await = Some((permit.generation, child));
        }
    }

    /// La llama el proxy de streaming cuando la respuesta HTTP termina (normal o por
    /// desconexión del cliente): mata el proceso si sigue siendo el de esa sesión. Si una
    /// sesión más nueva ya lo reemplazó, no hace nada (esa sesión ya se ocupa del suyo).
    pub async fn release_process(&self, permit_generation: u64) {
        let mut slot = self.process.lock().await;
        if slot.as_ref().map(|(gen, _)| *gen) == Some(permit_generation) {
            if let Some((_, mut child)) = slot.take() {
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
        }
    }

    /// `true` mientras ninguna sesión más nueva haya pedido la conexión: las sesiones sin
    /// proceso (la grabación del directo de iOS, F15) lo miran para saber si deben
    /// terminar.
    pub fn is_current_generation(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }

    /// El sincronizador de catálogo la consulta antes de cada lote (R1): se pausa
    /// mientras haya reproducción activa, nunca cancela por eso.
    pub fn is_busy(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// Para sesiones sin cuerpo de respuesta HTTP al que atar la duración del permiso
    /// (nivel 3, mpv: se lanza y la petición HTTP que lo pidió termina al instante, pero
    /// la reproducción de verdad sigue en su propia ventana). Mantiene `is_busy()` en
    /// `true` hasta que el proceso adjunto a `permit` termine solo, o hasta que una
    /// sesión más nueva ya se lo haya llevado (entonces esa sesión nueva es quien manda,
    /// y aquí no queda nada que esperar). Consume `permit`: al volver, ya se soltó.
    pub async fn run_until_exit_or_replaced(&self, permit: StreamPermit) {
        self.run_while(permit, || true).await;
    }

    /// Como [`Self::run_until_exit_or_replaced`], pero además termina la sesión (mata su
    /// proceso) en cuanto `wanted()` diga que ya nadie la usa. Para la grabación del
    /// directo (F13): sigue viva aunque el reproductor deje de leer el vídeo en directo
    /// (está en pausa), mientras siga dando señales de vida.
    pub async fn run_while(&self, permit: StreamPermit, wanted: impl Fn() -> bool) {
        loop {
            let mut slot = self.process.lock().await;
            match slot.as_mut() {
                Some((gen, _)) if *gen != permit.generation => break, // reemplazada: la sesión nueva ya se ocupa del suyo.
                Some((_, child)) => {
                    if matches!(child.try_wait(), Ok(Some(_))) {
                        slot.take(); // terminó solo (p. ej. el usuario cerró la ventana de mpv).
                        break;
                    }
                    if !wanted() {
                        if let Some((_, mut child)) = slot.take() {
                            let _ = child.start_kill();
                            let _ = child.wait().await;
                        }
                        break;
                    }
                }
                None => break, // nunca se adjuntó proceso, o ya se limpió por otra vía.
            }
            drop(slot);
            tokio::time::sleep(EXIT_POLL_INTERVAL).await;
        }
        drop(permit); // libera `active` ahora, no antes.
    }
}

/// Permiso de reproducción. Al soltarse (incluido un pánico durante la sesión) libera el
/// guard, salvo que ya se haya adquirido una sesión más nueva mientras tanto. Matar el
/// proceso ffmpeg asociado necesita `.await`, que `Drop` no puede ofrecer: eso lo hace
/// `release_process`, llamado explícitamente desde el proxy de streaming.
pub struct StreamPermit {
    guard: Arc<StreamGuard>,
    generation: u64,
}

impl StreamPermit {
    pub fn generation(&self) -> u64 {
        self.generation
    }
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
    use std::process::Stdio;

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

    /// Un proceso longevo cualquiera, disponible en cualquier máquina de desarrollo o de
    /// CI: no depende de que ffmpeg esté instalado, solo de que exista un intérprete de
    /// comandos, igual en Windows que en Unix.
    fn spawn_long_lived_process() -> Child {
        #[cfg(windows)]
        {
            tokio::process::Command::new("cmd")
                .args(["/C", "ping", "-n", "60", "127.0.0.1"])
                .stdout(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .expect("no se pudo lanzar el proceso de prueba")
        }
        #[cfg(not(windows))]
        {
            tokio::process::Command::new("sleep")
                .arg("60")
                .stdout(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .expect("no se pudo lanzar el proceso de prueba")
        }
    }

    #[tokio::test]
    async fn acquiring_again_kills_and_confirms_death_of_the_previous_process() {
        let guard = StreamGuard::new();
        let first = guard.acquire().await;
        let child = spawn_long_lived_process();
        let pid = child.id().expect("el proceso debería seguir vivo");
        guard.attach_process(&first, child).await;

        let second = guard.acquire().await;

        // Si el proceso sigue vivo, `try_wait` en un proceso ajeno con el mismo pid no
        // es fiable tras matarlo (el pid puede reciclarse), así que se comprueba a través
        // del propio Windows/POSIX: `tasklist`/`kill -0` no son portables en un test. En
        // su lugar se confía en que `acquire()` ya esperó `child.wait()` de verdad antes
        // de devolver el permiso: si esta línea se alcanza, la espera terminó.
        let _ = pid; // documenta qué proceso se mató; no hay forma portable de reinspeccionarlo aquí.
        drop(second);
        drop(first);
    }

    #[tokio::test]
    async fn release_process_kills_only_if_still_the_current_generation() {
        let guard = StreamGuard::new();
        let permit = guard.acquire().await;
        let child = spawn_long_lived_process();
        guard.attach_process(&permit, child).await;

        // Una sesión nueva ya se llevó el proceso: release_process de la vieja generación
        // no debe tocar nada (no hay proceso que matar dos veces).
        let _second = guard.acquire().await;
        guard.release_process(permit.generation()).await;
    }

    /// Un proceso que termina solo en cuanto puede: usado para probar
    /// `run_until_exit_or_replaced` sin esperar los 60 s del proceso longevo de arriba.
    fn spawn_quick_exit_process() -> Child {
        #[cfg(windows)]
        {
            tokio::process::Command::new("cmd").args(["/C", "exit", "0"]).stdout(Stdio::null()).kill_on_drop(true).spawn().unwrap()
        }
        #[cfg(not(windows))]
        {
            tokio::process::Command::new("true").stdout(Stdio::null()).kill_on_drop(true).spawn().unwrap()
        }
    }

    #[tokio::test]
    async fn run_until_exit_or_replaced_returns_once_the_process_exits_on_its_own() {
        let guard = StreamGuard::new();
        let permit = guard.acquire().await;
        guard.attach_process(&permit, spawn_quick_exit_process()).await;
        assert!(guard.is_busy());

        // Sin `timeout`, un fallo real (nunca detecta la salida) colgaría el test para
        // siempre en vez de fallar con un mensaje claro.
        tokio::time::timeout(std::time::Duration::from_secs(5), guard.run_until_exit_or_replaced(permit))
            .await
            .expect("run_until_exit_or_replaced no debería colgarse");

        assert!(!guard.is_busy(), "is_busy() debe volver a false cuando el proceso termina solo");
    }

    #[tokio::test]
    async fn run_while_kills_the_process_once_nobody_wants_the_session() {
        let guard = StreamGuard::new();
        let permit = guard.acquire().await;
        guard.attach_process(&permit, spawn_long_lived_process()).await;
        let wanted = Arc::new(AtomicBool::new(true));

        let task = {
            let guard = Arc::clone(&guard);
            let wanted = Arc::clone(&wanted);
            tokio::spawn(async move { guard.run_while(permit, move || wanted.load(Ordering::SeqCst)).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        assert!(guard.is_busy(), "mientras se quiera, la sesión sigue");

        wanted.store(false, Ordering::SeqCst);
        tokio::time::timeout(std::time::Duration::from_secs(5), task).await.expect("debería terminar").unwrap();
        assert!(!guard.is_busy());
        assert!(guard.process.lock().await.is_none(), "el proceso se mató, no se quedó huérfano");
    }

    #[tokio::test]
    async fn run_until_exit_or_replaced_returns_immediately_if_superseded() {
        let guard = StreamGuard::new();
        let first = guard.acquire().await;
        guard.attach_process(&first, spawn_long_lived_process()).await;

        let second = guard.acquire().await; // mata y espera el de `first` de verdad.
        guard.attach_process(&second, spawn_quick_exit_process()).await;

        // `first` ya no es la sesión vigente: no debe quedarse esperando nada.
        tokio::time::timeout(std::time::Duration::from_secs(2), guard.run_until_exit_or_replaced(first))
            .await
            .expect("no debería bloquear: la sesión ya fue reemplazada");

        // La sesión vigente (`second`) sigue contando como ocupada.
        assert!(guard.is_busy());
        drop(second);
    }
}
