# PRD — VIBE IPTV

## Problema

Los reproductores IPTV con buena experiencia de usuario son de pago y con suscripción
recurrente. Los gratuitos funcionan pero tienen interfaz de lista plana: nombres de
canal en texto, sin portadas, sin sinopsis, sin continuar viendo. El contenido es el
mismo; lo que falta es la capa de presentación.

## Solución

Un cliente propio que consume el proveedor IPTV del usuario y construye encima una
capa de catálogo enriquecida con metadatos de TMDB, con la navegación y el aspecto de
una plataforma de streaming comercial.

## Usuario

Una persona, en su casa, con una o dos suscripciones IPTV. Empieza en el PC. Más
adelante quiere lo mismo en el televisor a través de un Fire TV Stick, y opcionalmente
en el móvil. No tiene servidor propio ni quiere administrarlo.

## Fuentes de contenido soportadas

1. **Xtream Codes** (host, usuario, contraseña). Es la fuente preferente: devuelve JSON
   estructurado con categorías, series desglosadas en temporadas y episodios, e
   información de la cuenta incluido el límite de conexiones.
2. **Lista M3U + XMLTV** (URL o fichero). Fuente de respaldo para proveedores que solo
   dan un enlace. Todo se deduce de los atributos de `#EXTINF`.

Ambas conviven: el usuario puede tener varios proveedores configurados a la vez, y el
catálogo se presenta unificado con deduplicación por título.

> Nota: un enlace `get.php?username=...&password=...&type=m3u_plus` ya contiene las
> credenciales Xtream. Al añadir una fuente M3U, la app intenta detectarlo y ofrece
> convertirla a fuente Xtream, que da mucha más información. **Nunca se usan las dos
> variantes del mismo proveedor a la vez**: contarían como dos conexiones.

---

## Requisitos funcionales

### RF1 — Configuración de fuentes
- Alta de proveedor Xtream (host, usuario, contraseña) con validación contra
  `player_api.php` y muestra de estado de cuenta, caducidad y conexiones máximas.
- Alta de fuente M3U por URL o fichero local.
- Detección de credenciales embebidas en URL M3U y conversión a Xtream.
- Editar, desactivar y eliminar fuentes.

### RF2 — Sincronización de catálogo
- Sincronización completa a demanda y automática al arrancar si han pasado más de 12 h.
- Incremental: se comparan identificadores remotos y solo se escriben los cambios.
- Progreso visible con posibilidad de cancelar.
- Se pausa automáticamente mientras hay reproducción activa.

### RF3 — Normalización y enriquecimiento
- Limpieza de títulos: prefijos de idioma o país, marcas de calidad, etiquetas de
  release, extracción de año.
- Emparejamiento con TMDB por título y año; caché persistente del resultado, incluidos
  los fallos, para no repetir consultas.
- Descarga y caché en disco de portada y fondo.
- Detección de duplicados entre calidades del mismo título (SD/HD/FHD/4K), agrupados en
  una sola ficha con selector de calidad.

### RF4 — Navegación del catálogo
- **Inicio**: continuar viendo, añadidos recientemente, filas por género, favoritos.
- **Directo**: lista de canales por categoría, con logo, programa actual y barra de
  progreso del programa.
- **Películas** y **Series**: rejilla con portadas, filtros por género, año y calidad,
  y ordenación.
- **Ficha de detalle**: fondo, sinopsis, año, duración, valoración, géneros, botón de
  reproducir o reanudar. En series, selector de temporada y lista de episodios con
  progreso individual.
- **Buscador** instantáneo sobre todo el catálogo, con resultados agrupados por tipo.

### RF5 — Reproducción
- Reproducción de directo, película y episodio con la cascada de tres niveles descrita
  en `ARCHITECTURE.md`.
- Controles superpuestos: play/pausa, barra de progreso con búsqueda, volumen, pista de
  audio, subtítulos, calidad, pantalla completa.
- Guardado de posición cada 10 s; se considera terminado a partir del 90%.
- Siguiente episodio automático al terminar, con cuenta atrás cancelable.
- Zapping en directo con las flechas arriba y abajo, objetivo por debajo de 2 s.
- Estadísticas de reproducción ocultas tras una combinación de teclas, para diagnóstico.

### RF6 — Guía de programación
- Importación de XMLTV, propio del proveedor o de una URL externa.
- Vista de rejilla temporal por canal.
- Programa actual y siguiente en la lista de canales.
- Catch-up y timeshift cuando el proveedor lo soporte.

### RF7 — Estado del usuario
- Favoritos de canal, película y serie.
- Historial de reproducción con posiciones.
- Perfiles: uno por defecto en v1, con el modelo de datos ya preparado para varios.
- PIN de control parental por categoría, en v1 solo para ocultar categorías adultas.

---

## Requisitos no funcionales

| Métrica | Objetivo |
|---|---|
| Arranque en frío hasta ver la pantalla de inicio | < 2 s |
| Sincronización completa de 50.000 entradas | < 60 s |
| Búsqueda con resultados en pantalla | < 100 ms |
| Zapping entre canales de directo | < 2 s |
| Consumo de RAM en reposo | < 400 MB |
| Tamaño del instalador | < 60 MB con sidecars |
| Conexiones simultáneas al proveedor | máximo 1 |

---

## Fuera de alcance en v1

- Grabación programada de contenido.
- Descarga de contenido para ver sin conexión.
- Reproducción compartida o sincronizada entre dispositivos.
- Cualquier forma de DRM.
- Publicación en tiendas de aplicaciones.
- Compartir el catálogo con usuarios que no sean el propietario de la suscripción.

## Criterio de éxito

El usuario deja de abrir su aplicación IPTV de pago porque esta es más agradable de usar.
