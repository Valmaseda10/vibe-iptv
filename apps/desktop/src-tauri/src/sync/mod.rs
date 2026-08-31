//! Sincronización opcional de perfiles/favoritos/progreso contra Supabase (excepción
//! consciente a la regla R2 de `CLAUDE.md` — ver `docs/DECISIONS.md` para el porqué y
//! los límites exactos). El catálogo, las credenciales de proveedor y el streaming en
//! sí **nunca** pasan por aquí: siguen siendo cosa exclusiva de este PC.

pub mod engine;
pub mod session;
pub mod supabase;
