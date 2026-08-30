//! Envoltorio fino sobre `keyring`. Las contraseñas de Xtream y las URLs firmadas de
//! M3U viven aquí, nunca en SQLite (regla R5 de CLAUDE.md).

const SERVICE: &str = "vibe-iptv";

#[derive(Debug, thiserror::Error)]
pub enum KeychainError {
    #[error("no se pudo acceder al almacén de credenciales del sistema")]
    Backend(#[from] keyring::Error),
}

type Result<T> = std::result::Result<T, KeychainError>;

/// Guarda `secret` bajo `credential_ref`, sobrescribiendo cualquier valor anterior.
pub fn store_secret(credential_ref: &str, secret: &str) -> Result<()> {
    keyring::Entry::new(SERVICE, credential_ref)?.set_password(secret)?;
    Ok(())
}

pub fn read_secret(credential_ref: &str) -> Result<String> {
    Ok(keyring::Entry::new(SERVICE, credential_ref)?.get_password()?)
}

pub fn delete_secret(credential_ref: &str) -> Result<()> {
    keyring::Entry::new(SERVICE, credential_ref)?.delete_credential()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Estos tests tocan el almacén de credenciales real del sistema operativo. En
    // Windows (Credential Manager) funcionan sin configuración; si CI no tiene backend
    // disponible, fallarán por KeychainError::Backend y hay que verificarlo a mano.
    fn test_ref(suffix: &str) -> String {
        format!("test-{}-{}", std::process::id(), suffix)
    }

    #[test]
    fn stores_and_reads_back_a_secret() {
        let credential_ref = test_ref("roundtrip");
        store_secret(&credential_ref, "hunter2").unwrap();
        assert_eq!(read_secret(&credential_ref).unwrap(), "hunter2");
        delete_secret(&credential_ref).unwrap();
    }

    #[test]
    fn overwriting_replaces_the_previous_value() {
        let credential_ref = test_ref("overwrite");
        store_secret(&credential_ref, "primero").unwrap();
        store_secret(&credential_ref, "segundo").unwrap();
        assert_eq!(read_secret(&credential_ref).unwrap(), "segundo");
        delete_secret(&credential_ref).unwrap();
    }

    #[test]
    fn reading_a_missing_secret_fails() {
        let credential_ref = test_ref("missing");
        assert!(read_secret(&credential_ref).is_err());
    }
}
