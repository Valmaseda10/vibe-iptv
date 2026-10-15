//! Sincronización de perfiles, favoritos y progreso entre dispositivos, contra Firebase
//! (F11, sustituye a Supabase). Excepción consciente a la regla R2 de `CLAUDE.md`, con
//! límites estrictos: el catálogo, las credenciales de proveedor y el streaming **nunca**
//! pasan por aquí, y la app funciona igual sin conexión con la nube. Ver
//! `docs/CLOUD-SYNC.md`.

pub mod backend;
pub mod engine;
pub mod firebase;
#[cfg(test)]
pub mod memory;
pub mod outbox;
pub mod session;
