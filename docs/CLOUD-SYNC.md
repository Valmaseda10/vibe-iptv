# Sincronización en la nube — VIBE IPTV

Perfiles, favoritos y progreso de reproducción viajan entre dispositivos a través de
Firebase (F11; antes Supabase). Es la única excepción a la regla R2 de `CLAUDE.md`, con
límites que no se negocian:

- **Nunca pasan por la nube** el catálogo, las credenciales del proveedor IPTV, las URLs
  de stream ni nada que identifique al proveedor (R5).
- **La app nunca depende de la nube para funcionar.** Sin red, con Firebase caído o con
  la cuenta sin conexión, todo sigue igual en el PC; los cambios esperan en una cola.
- **Los canales de directo no se sincronizan** (ni favoritos ni progreso): no tienen una
  clave estable entre proveedores distintos.

---

## Por qué Firebase

| Opción | Motivo del descarte |
|---|---|
| Supabase gratuito | Pausa el proyecto tras ~7 días sin uso. Con la cuenta obligatoria, la app quedaba inutilizable hasta reactivarlo a mano. |
| Supabase Pro | 25 $/mes por tres tablas pequeñas. |
| Neon (Postgres + Data API + Auth) | Su Auth está en beta, no permite cambiar contraseña con el SDK gestionado y la recuperación por correo exige SMTP propio. |
| Cloudflare Workers + D1 | Auth y envío de correos propios: superficie de seguridad que mantener. |

Firebase no pausa proyectos por inactividad, su Auth por REST es estable y envía los
correos de recuperación, y tiene SDK nativo con caché offline para iOS y Android (F15,
F16). Plan gratuito (Spark): 50.000 lecturas y 20.000 escrituras al día, de sobra con la
agrupación de escrituras descrita abajo.

---

## Proyecto

- Proyecto: `vibe-iptv-sync`. Firestore en `europe-southwest1` (Madrid), modo nativo.
- Auth: solo correo y contraseña. Protección contra enumeración de correos activa: el
  login falla con `INVALID_LOGIN_CREDENTIALS` sin decir si el correo existe, y pedir
  recuperación para un correo sin cuenta responde igual que uno con cuenta.
- La API key web va en el código (`sync/firebase.rs`). No es un secreto: identifica el
  proyecto. Los datos los protegen las reglas.
- Toda la configuración vive en `infra/firebase/` y se despliega con la CLI:

```
cd infra/firebase
firebase deploy --only firestore,auth
```

### Reglas de seguridad

```
match /users/{uid}/{document=**} {
  allow read, write: if request.auth != null && request.auth.uid == uid;
}
```

Cada cuenta solo lee y escribe bajo su `users/{uid}`. Comprobado contra el proyecto real:
otro uid y una petición sin token reciben 403. Las reglas tardan alrededor de un minuto
en propagarse tras desplegarlas.

---

## Modelo de documentos

Tres colecciones planas bajo cada usuario. Planas y no anidadas (favoritos dentro de su
perfil) para que la bajada incremental sea una consulta simple por colección, sin
consultas de grupo de colecciones ni índices compuestos.

```
users/{uid}/profiles/{profile_id}
users/{uid}/favorites/{sha256(target)}
users/{uid}/playback/{sha256(target)}
```

| Campo | Colecciones | Notas |
|---|---|---|
| `name`, `avatar`, `adult_ok`, `pin_hash` | profiles | `pin_hash` es SHA-256 del PIN, nunca el PIN |
| `profile_id` | favorites, playback | id del documento de perfil |
| `item_kind` | favorites, playback | `movie`, `series` o `episode` |
| `item_key` | favorites, playback | `match_key` de película o serie; `match_key:temporada:número` para episodios |
| `added_at` | favorites | epoch en segundos |
| `position_s`, `duration_s`, `finished`, `played_at` | playback | `finished` a partir del 90 % |
| `updated_at` | todas | epoch del cliente que hizo el cambio: decide quién gana un conflicto |
| `deleted` | todas | borrado lógico (ver abajo) |
| `server_updated_at` | todas | marca de tiempo del servidor (`REQUEST_TIME`): ordena la bajada |

- **Ids.** Un perfil usa su propio id: 20 caracteres alfanuméricos generados en el
  cliente (así se puede encolar sin conexión), o el UUID que le dio Supabase si viene de
  antes de F11. Favoritos y progreso usan el SHA-256 en hexadecimal de su `target`
  (`favorite:{perfil}:{tipo}:{clave}`), porque `item_key` puede llevar `/` y no tiene
  longitud acotada.
- **Borrados lógicos.** Nada se borra de verdad en Firestore: se escribe el documento con
  `deleted = true`. Un borrado real no aparecería en la bajada incremental y el resto de
  dispositivos nunca se enterarían.
- **Enteros.** Firestore los representa como cadena en JSON (`{"integerValue": "600"}`).
  Un cliente que escriba `doubleValue` también se lee bien.

---

## Cómo funciona en el PC

```
acción del usuario ──► SQLite (cambio + entrada en sync_outbox, misma conexión)
                                   │
                    motor en segundo plano (sync::engine)
                    ├─ 1. primera vez con esta cuenta: fusión completa
                    ├─ 2. envía la cola (commit de hasta 200 documentos)
                    └─ 3. cada 3 min: baja lo cambiado desde el cursor
```

### Cola de salida (`sync_outbox`, migración 011)

- Cada cambio local se encola **en la misma conexión** que lo guarda (`sync::outbox`). Lo
  encolado es una foto del estado local en ese momento: si la fila existe, el documento;
  si ya no existe, un borrado lógico.
- Una fila por documento remoto (`target` único): un cambio nuevo sustituye al pendiente.
  `revision` evita perder un cambio que llega mientras el anterior estaba en vuelo.
- El progreso del reproductor (cada 10 s) se encola con un retraso de 60 s, y como se va
  sustituyendo, a la nube llega como mucho una escritura por título y minuto.
- Tras un fallo: 30 s, 1 min, 2 min... hasta una hora entre intentos. Nunca se descarta
  una entrada por fallar; solo al cerrar sesión.
- El motor despierta cada 30 s, o al momento cuando se encola algo o se inicia sesión.

### Bajada incremental

- Por colección, documentos con `server_updated_at` posterior al cursor guardado en
  `app_settings` (`cloud.cursor.<colección>`), paginados de 300 en 300 con cursor
  `(server_updated_at, __name__)`.
- Se releen 2 minutos antes del cursor: una escritura con marca anterior puede hacerse
  visible un instante después. Aplicar dos veces el mismo documento no cambia nada.

### Reglas de fusión (`apply_remote`)

1. Un cambio local todavía en la cola gana siempre: se enviará y sobrescribirá.
2. Si no, gana el `updated_at` más reciente (para el progreso, `played_at`).
3. Lo bajado conserva sus fechas originales: un episodio visto hace semanas en otro
   dispositivo no salta al principio de "Continuar viendo".
4. Lo que el catálogo de este PC no tiene (otro proveedor) se ignora.
5. Nunca se borra el último perfil local, aunque llegue su borrado.
6. Al llegar el borrado de un perfil, el PC borra el perfil local con su cascada; sus
   favoritos y progreso no necesitan borrados propios.

### Primera fusión con una cuenta

Cuando una cuenta se usa por primera vez en un PC (`cloud.merged_uid` distinto), el motor
baja todo, lo aplica y encola lo local que la nube no tiene o tiene más viejo. Esto cubre
la migración desde Supabase: la cuenta de Firebase empieza vacía y recibe todo lo del PC,
**incluido el progreso**, que con Supabase nunca llegaba a subirse.

**Los perfiles del PC tienen dueño** (`account.data_owner_uid`, que sobrevive al cierre
de sesión). Al iniciar sesión, y otra vez antes de fusionar, si los perfiles locales son
de otra cuenta se borran del PC (con sus favoritos, progreso y cola pendiente) en vez de
subirse a la nueva. Volver con la misma cuenta no pierde nada, y los perfiles sin dueño
(creados antes de iniciar sesión nunca) pasan a ser de la cuenta que entra. Antes de
esto, cerrar sesión y entrar con otra cuenta le subía los perfiles de la anterior
(encontrado al pasar de la cuenta de pruebas a la del usuario).

### Sesión y local-first

- El keychain guarda solo el token de refresco, el uid y el correo. El token de acceso
  (1 h) vive en memoria: una sesión completa no cabe en el límite de 2560 bytes del
  Administrador de credenciales de Windows.
- La sesión solo se borra si Firebase rechaza el token de refresco. Sin red o con el
  servidor caído se sigue usando la que hay.
- `GET /api/account/status` no toca la red. Un fallo pasajero en login o registro
  responde 503, y la pantalla de cuenta ofrece "Entrar sin conexión".
- Cerrar sesión intenta enviar lo pendiente durante 5 s y luego vacía la cola y los
  cursores: eran de esa cuenta. Los perfiles se quedan en el PC hasta saber quién entra
  (ver "Primera fusión con una cuenta").

---

## Para los clientes iOS y Fire TV (F15, F16)

Usarán el SDK oficial de Firebase contra este mismo modelo. Lo imprescindible:

- Escribir siempre `server_updated_at` con `FieldValue.serverTimestamp()`.
- Nunca borrar documentos: `deleted = true` y `updated_at` con la hora del borrado.
- `updated_at`, `played_at` y `added_at` en **segundos** epoch, como enteros.
- Ids de favoritos y progreso: SHA-256 en hexadecimal de `favorite:{perfil}:{tipo}:{clave}`
  o `playback:{perfil}:{tipo}:{clave}`, igual que el PC, o el mismo favorito se duplicaría.
- `item_key` sale del `match_key` que calcula el PC (`catalog::normalize`). Un cliente que
  no tenga el catálogo del PC tiene que pedírselo por la API LAN, no recalcularlo.

---

## Verificar contra el proyecto real

Con una cuenta de pruebas (las credenciales por entorno, nunca en el repositorio):

```
cd apps/desktop/src-tauri
VIBE_QA_EMAIL=... VIBE_QA_PASSWORD=... cargo test live_ -- --ignored --test-threads=1
```

- `sync::firebase::tests::live_roundtrip_against_the_real_project`: login, refresco,
  escritura, bajada incremental, borrado lógico y error de contraseña.
- `server::tests::live_end_to_end_sync_through_the_local_api`: API HTTP local → cola →
  motor en segundo plano → Firestore, con una base de datos temporal (nunca la real).

---

## Límites conocidos

- **Lo que el catálogo de este PC no tiene al bajarlo se pierde para este PC** hasta la
  siguiente primera fusión (otro inicio de sesión). Si el proveedor añade después ese
  título, su favorito o progreso no aparece solo.
- **Relojes.** Los conflictos se resuelven con el reloj del cliente que hizo el cambio. Un
  dispositivo con la hora muy mal puede ganar o perder un conflicto que no debería.
- **Perfiles compartidos entre cuentas.** La tabla local `profiles` es del PC, no de la
  cuenta: iniciar sesión con otra cuenta en el mismo PC sube los mismos perfiles locales.
- **Cuotas.** Una familia normal queda muy lejos de 20.000 escrituras al día, pero no hay
  ninguna alarma si se acercara. Firebase corta las escrituras hasta el día siguiente; la
  cola las guardaría y reintentaría.
