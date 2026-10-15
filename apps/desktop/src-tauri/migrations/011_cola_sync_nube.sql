-- F11: la nube pasa de Supabase a Firebase y la app se vuelve local-first (ver
-- docs/CLOUD-SYNC.md). Todo cambio local que deba llegar a la nube se apunta aquí en la
-- misma transacción que el propio cambio, y un proceso en segundo plano lo envía con
-- reintentos. Antes cada cambio se enviaba una sola vez y, si la nube no respondía, se
-- perdía para siempre.

CREATE TABLE sync_outbox (
  id              INTEGER PRIMARY KEY,
  -- Documento remoto al que afecta ('profile:<id>', 'favorite:<perfil>:<tipo>:<clave>',
  -- 'playback:<perfil>:<tipo>:<clave>'). UNIQUE: un cambio nuevo sobre el mismo
  -- documento sustituye al pendiente en vez de acumularse (el progreso de reproducción
  -- se guarda cada 10 s; a la nube solo tiene que llegar el último).
  target          TEXT NOT NULL UNIQUE,
  collection      TEXT NOT NULL CHECK (collection IN ('profiles','favorites','playback')),
  payload         TEXT NOT NULL,           -- JSON del documento remoto completo
  -- Sube cada vez que un cambio nuevo sustituye al pendiente: el envío solo borra la
  -- fila si sigue en la misma revisión que mandó, así no se pierde un cambio que llegó
  -- mientras la petición estaba en vuelo.
  revision        INTEGER NOT NULL DEFAULT 1,
  attempts        INTEGER NOT NULL DEFAULT 0,
  next_attempt_at INTEGER NOT NULL,
  last_error      TEXT,
  created_at      INTEGER NOT NULL
);
CREATE INDEX idx_sync_outbox_due ON sync_outbox(next_attempt_at);
