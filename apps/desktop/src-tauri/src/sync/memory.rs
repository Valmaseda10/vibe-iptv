//! Nube falsa en memoria para los tests del motor de sincronización: guarda documentos
//! con una marca de tiempo de servidor propia, y puede simular cortes y cambios hechos
//! desde otro dispositivo. Solo existe en tests.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use super::backend::{Changes, CloudBackend, CloudError, Collection, RemoteDoc, Result, Session};

/// Cada escritura avanza el reloj del servidor 1000 s: así un test sabe sin ambigüedad
/// qué es nuevo respecto a un cursor, aunque el motor relea un margen hacia atrás.
const CLOCK_STEP: i64 = 1_000;

struct State {
    docs: HashMap<(Collection, String), (i64, RemoteDoc)>,
    clock: i64,
    fail_everything: Option<CloudError>,
    fail_refresh: Option<CloudError>,
    write_calls: usize,
    refreshes: usize,
}

pub struct MemoryBackend {
    state: Mutex<State>,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                docs: HashMap::new(),
                clock: 1_700_000_000,
                fail_everything: None,
                fail_refresh: None,
                write_calls: 0,
                refreshes: 0,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Simula la nube caída (o sin red): toda llamada falla con este error.
    pub fn fail_everything(&self, err: Option<CloudError>) {
        self.lock().fail_everything = err;
    }

    pub fn fail_refresh(&self, err: Option<CloudError>) {
        self.lock().fail_refresh = err;
    }

    /// Un cambio hecho desde otro dispositivo, como si lo hubiera escrito él.
    pub fn put_from_other_device(&self, doc: RemoteDoc) {
        let mut state = self.lock();
        state.clock += CLOCK_STEP;
        let ts = state.clock;
        state.docs.insert((doc.collection(), doc.doc_id()), (ts, doc));
    }

    pub fn docs(&self, collection: Collection) -> Vec<RemoteDoc> {
        let state = self.lock();
        let mut docs: Vec<(i64, RemoteDoc)> = state.docs.iter().filter(|((c, _), _)| *c == collection).map(|(_, v)| v.clone()).collect();
        docs.sort_by_key(|(ts, _)| *ts);
        docs.into_iter().map(|(_, doc)| doc).collect()
    }

    pub fn write_calls(&self) -> usize {
        self.lock().write_calls
    }

    pub fn session(&self) -> Session {
        Session {
            access_token: format!("token-{}", self.lock().refreshes),
            refresh_token: "refresh".into(),
            user_id: "uid-test".into(),
            email: "qa@example.com".into(),
            expires_at: crate::db::now_epoch() + 3600,
        }
    }

    fn check(&self) -> Result<()> {
        match &self.lock().fail_everything {
            Some(err) => Err(err.clone()),
            None => Ok(()),
        }
    }
}

#[async_trait]
impl CloudBackend for MemoryBackend {
    async fn sign_up(&self, _email: &str, _password: &str) -> Result<Session> {
        self.check()?;
        Ok(self.session())
    }

    async fn sign_in(&self, _email: &str, _password: &str) -> Result<Session> {
        self.check()?;
        Ok(self.session())
    }

    async fn refresh(&self, _session: &Session) -> Result<Session> {
        self.check()?;
        if let Some(err) = self.lock().fail_refresh.clone() {
            return Err(err);
        }
        self.lock().refreshes += 1;
        Ok(self.session())
    }

    async fn send_password_reset(&self, _email: &str) -> Result<()> {
        self.check()
    }

    async fn confirm_password_reset(&self, _code: &str, _new_password: &str) -> Result<()> {
        self.check()
    }

    async fn change_password(&self, _session: &Session, _new_password: &str) -> Result<Session> {
        self.check()?;
        Ok(self.session())
    }

    async fn write(&self, _session: &Session, docs: &[RemoteDoc]) -> Result<()> {
        self.check()?;
        let mut state = self.lock();
        state.write_calls += 1;
        state.clock += CLOCK_STEP;
        let ts = state.clock;
        for doc in docs {
            state.docs.insert((doc.collection(), doc.doc_id()), (ts, doc.clone()));
        }
        Ok(())
    }

    async fn changes_since(&self, _session: &Session, collection: Collection, since: i64) -> Result<Changes> {
        self.check()?;
        let state = self.lock();
        let mut found: Vec<(i64, RemoteDoc)> =
            state.docs.iter().filter(|((c, _), (ts, _))| *c == collection && *ts > since).map(|(_, v)| v.clone()).collect();
        found.sort_by_key(|(ts, _)| *ts);
        Ok(Changes { max_server_ts: found.last().map(|(ts, _)| *ts), docs: found.into_iter().map(|(_, doc)| doc).collect() })
    }
}
