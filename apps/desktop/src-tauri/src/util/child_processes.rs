//! Los procesos hijos (ffmpeg, ffprobe, mpv) mueren con la app, también si se cierra de
//! golpe o se cuelga.
//!
//! `kill_on_drop` solo actúa si el `Child` llega a soltarse, y al cerrar la ventana el
//! proceso termina sin pasar por ahí. En Windows un hijo no muere con su padre: un ffmpeg
//! que estuviera grabando el directo para la pausa (escribe en disco, no en una tubería)
//! seguiría vivo, con la única conexión que admite la cuenta del proveedor (regla R1).
//! En Windows se mete la app en un «job» que mata a todo su grupo al cerrarse; en el resto
//! de sistemas no hace falta (iOS no lanza procesos).

/// Al arrancar: a partir de aquí, todo lo que lance la app se cierra con ella.
pub fn tie_to_app_lifetime() {
    #[cfg(windows)]
    match windows_job::assign_current_process() {
        Ok(_) => tracing::debug!("los procesos hijos se cerrarán con la app"),
        Err(code) => tracing::warn!(code, "no se pudo atar los procesos hijos a la app"),
    }
}

#[cfg(windows)]
mod windows_job {
    use std::mem::{size_of, zeroed};
    use std::ptr::null;

    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    /// Crea el job, le pone «matar todo al cerrar» y mete en él a la propia app. El handle
    /// del job no se cierra nunca a mano: lo cierra el sistema cuando la app termina, y
    /// es eso lo que mata a los hijos. Devuelve el job, o el código de error de Windows.
    pub fn assign_current_process() -> Result<HANDLE, u32> {
        // SAFETY: llamadas a la API de Windows con punteros nulos permitidos (sin nombre ni
        // atributos de seguridad), una estructura del tamaño exacto que se declara, y el
        // pseudo-handle del proceso actual, que no hay que cerrar.
        unsafe {
            let job = CreateJobObjectW(null(), null());
            if job.is_null() {
                return Err(last_error());
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const core::ffi::c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if set == 0 {
                return Err(last_error());
            }
            if AssignProcessToJobObject(job, GetCurrentProcess()) == 0 {
                return Err(last_error());
            }
            Ok(job)
        }
    }

    fn last_error() -> u32 {
        // SAFETY: sin argumentos; solo lee el último error del hilo.
        unsafe { windows_sys::Win32::Foundation::GetLastError() }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use windows_sys::Win32::Foundation::{CloseHandle, BOOL};
    use windows_sys::Win32::System::JobObjects::IsProcessInJob;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    /// Un hijo lanzado después queda dentro del job: así muere con la app (que lo mate al
    /// cerrarse lo garantiza Windows con `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`).
    #[test]
    fn a_child_started_afterwards_belongs_to_the_app_job() {
        let job = super::windows_job::assign_current_process().expect("job");
        let mut child = std::process::Command::new("cmd").args(["/C", "ping -n 3 127.0.0.1 > NUL"]).spawn().expect("cmd");
        let mut in_job: BOOL = 0;
        // SAFETY: handle abierto aquí y cerrado justo después; `job` sigue abierto.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, child.id());
            assert!(!handle.is_null());
            assert_ne!(IsProcessInJob(handle, job, &mut in_job), 0);
            CloseHandle(handle);
        }
        let _ = child.kill();
        assert_ne!(in_job, 0, "el hijo tiene que estar en el job de la app");
    }
}
