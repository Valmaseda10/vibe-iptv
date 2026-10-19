// Proveedor Xtream falso para probar la app de punta a punta sin un proveedor real ni
// credenciales reales (regla R5). Sin dependencias: Node y ffmpeg en el PATH.
//
//   node tools/proveedor-falso/server.mjs
//
// En la app: Ajustes → Añadir proveedor → Xtream, URL http://127.0.0.1:8899,
// usuario "qa", contraseña "qa". Ver README.md.

import { createServer } from "node:http";
import { spawnSync } from "node:child_process";
import { createReadStream, existsSync, mkdirSync, statSync, openSync, readSync, closeSync, writeFileSync } from "node:fs";
import { gzipSync } from "node:zlib";
import { tmpdir } from "node:os";
import { join } from "node:path";

const PORT = Number(process.env.PORT ?? 8899);
const USER = "qa";
const PASS = "qa";
const BASE = `http://127.0.0.1:${PORT}`;
const MEDIA_DIR = join(tmpdir(), "vibe-proveedor-falso");
const now = Math.floor(Date.now() / 1000);
const DAY = 86_400;

// ─────────────── Medios generados con ffmpeg (una vez, se cachean en %TEMP%) ───────────────

const MEDIA = {
  // Directo: MPEG-TS H.264 + AAC, 10 minutos, baja resolución para generarlo rápido.
  live: { file: "directo.ts", seconds: 600, args: ["-c:v", "libx264", "-preset", "ultrafast", "-g", "50", "-b:v", "600k", "-c:a", "aac", "-b:a", "96k", "-f", "mpegts"] },
  // Directo con reloj: `testsrc` pinta los segundos y el audio es MP2 (como Telecinco).
  // Media hora, para probar a pausar 10 minutos y ver que se retoma en el segundo exacto.
  liveClock: { file: "directo-reloj.ts", seconds: 1800, source: "testsrc", args: ["-c:v", "libx264", "-preset", "ultrafast", "-g", "50", "-b:v", "400k", "-ac", "2", "-c:a", "mp2", "-b:a", "128k", "-f", "mpegts"] },
  // Directo con audio AC3: obliga al remux a recodificar el audio a AAC.
  liveAc3: { file: "directo-ac3.ts", seconds: 300, args: ["-c:v", "libx264", "-preset", "ultrafast", "-g", "50", "-b:v", "600k", "-c:a", "ac3", "-b:a", "192k", "-f", "mpegts"] },
  // Película MP4 H.264 + AAC con moov al principio: nivel 1 (proxy con Range).
  mp4: { file: "pelicula.mp4", seconds: 180, args: ["-c:v", "libx264", "-preset", "ultrafast", "-b:v", "800k", "-c:a", "aac", "-b:a", "128k", "-movflags", "+faststart", "-f", "mp4"] },
  // Película MKV con dos pistas de audio y dos de subtítulos SRT: nivel 2 (remux) y los
  // selectores de pista. `testsrc` pinta un contador de segundos, y cada frase dice en qué
  // segundo debería verse: así se comprueba a ojo que los subtítulos van a tiempo.
  mkv: { file: "pelicula-capitulos.mkv", seconds: 180, source: "testsrc", twoAudio: true, subtitles: true, chapters: true, args: ["-c:v", "libx264", "-preset", "ultrafast", "-b:v", "800k", "-c:a", "aac", "-b:a", "128k", "-c:s", "srt", "-f", "matroska"] },
  // Película en H.265/HEVC: la WebView sin decodificación HEVC no la abre (nivel 3, mpv).
  hevc: { file: "pelicula-hevc.mkv", seconds: 60, args: ["-c:v", "libx265", "-preset", "ultrafast", "-x265-params", "log-level=error", "-c:a", "aac", "-b:a", "128k", "-f", "matroska"] },
  // Película en H.264 de 10 bits (habitual en anime): la sonda solo ve "h264" y la manda al
  // remux. La WebView2 de Windows la decodifica (medido en F12); queda para comprobar que
  // sigue siendo así, porque si no, nada la mandaría a mpv.
  hi10: { file: "pelicula-h264-10bit.mkv", seconds: 60, args: ["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p10le", "-c:a", "aac", "-b:a", "128k", "-f", "matroska"] },
};

function ensureMedia() {
  mkdirSync(MEDIA_DIR, { recursive: true });
  for (const [name, spec] of Object.entries(MEDIA)) {
    const out = join(MEDIA_DIR, spec.file);
    if (existsSync(out)) continue;
    console.log(`generando ${spec.file} (${spec.seconds} s)…`);
    const inputs = [
      "-f", "lavfi", "-i", `${spec.source ?? "testsrc2"}=size=640x360:rate=25:duration=${spec.seconds}`,
      "-f", "lavfi", "-i", `sine=frequency=440:sample_rate=48000:duration=${spec.seconds}`,
    ];
    const maps = ["-map", "0:v", "-map", "1:a"];
    if (spec.twoAudio) {
      inputs.push("-f", "lavfi", "-i", `sine=frequency=880:sample_rate=48000:duration=${spec.seconds}`);
      maps.push("-map", "2:a", "-metadata:s:a:0", "language=spa", "-metadata:s:a:1", "language=eng");
    }
    if (spec.subtitles) {
      const first = inputs.filter((arg) => arg === "-i").length;
      for (const [lang, word] of [["spa", "Segundo"], ["eng", "Second"]]) {
        const srt = join(MEDIA_DIR, `subtitulos-${lang}.srt`);
        writeFileSync(srt, srtEvery(5, spec.seconds, (t) => `${word} ${t}`));
        inputs.push("-i", srt);
      }
      maps.push("-map", `${first}:s`, "-map", `${first + 1}:s`, "-metadata:s:s:0", "language=spa", "-metadata:s:s:1", "language=eng");
    }
    if (spec.chapters) {
      // Capítulos como los de un episodio de verdad: resumen, intro y el resto. El
      // reproductor solo ofrece "Saltar intro" porque uno se llama así.
      const metadata = join(MEDIA_DIR, "capitulos.txt");
      writeFileSync(
        metadata,
        ";FFMETADATA1\n" +
          "[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=20000\ntitle=Resumen\n" +
          "[CHAPTER]\nTIMEBASE=1/1000\nSTART=20000\nEND=50000\ntitle=Intro\n" +
          `[CHAPTER]\nTIMEBASE=1/1000\nSTART=50000\nEND=${spec.seconds * 1000}\ntitle=Capítulo\n`,
      );
      const index = inputs.filter((arg) => arg === "-i").length;
      inputs.push("-i", metadata);
      maps.push("-map_chapters", String(index));
    }
    const result = spawnSync("ffmpeg", ["-hide_banner", "-loglevel", "error", "-y", ...inputs, ...maps, ...spec.args, out], { stdio: "inherit" });
    if (result.status !== 0) {
      console.error(`ffmpeg falló generando ${name}; ¿está ffmpeg en el PATH?`);
      process.exit(1);
    }
  }
}

/** Una frase de 3 s cada `step` segundos, con el texto que dé `text(segundo)`. */
function srtEvery(step, seconds, text) {
  const time = (t) => `${String(Math.floor(t / 3600)).padStart(2, "0")}:${String(Math.floor((t % 3600) / 60)).padStart(2, "0")}:${String(t % 60).padStart(2, "0")},000`;
  const cues = [];
  for (let t = step, n = 1; t + 3 <= seconds; t += step, n++) cues.push(`${n}\n${time(t)} --> ${time(t + 3)}\n${text(t)}\n`);
  return cues.join("\n");
}

// ─────────────── Catálogo sintético ───────────────

const liveCategories = [
  { category_id: "1", category_name: "ESPAÑA | GENERALISTAS", parent_id: 0 },
  { category_id: "2", category_name: "DEPORTES", parent_id: 0 },
  { category_id: "3", category_name: "NOTICIAS", parent_id: 0 },
  { category_id: "4", category_name: "INFANTIL", parent_id: 0 },
  { category_id: "9", category_name: "XXX ADULTOS", parent_id: 0 },
];

// Nombres con el ruido típico de un panel real: prefijos de país, calidades y feeds
// duplicados del mismo canal (deben agruparse con selector de fuente).
const channelSpecs = [
  ["ES| La 1 HD", "1", "la1.es", true], ["ES| La 1 FHD", "1", "la1.es", true], ["ES| La 2", "1", "la2.es", false],
  ["ES| Antena 3 HD", "1", "antena3.es", true], ["ES| Cuatro", "1", "cuatro.es", false], ["ES| Telecinco HD", "1", "telecinco.es", true],
  ["ES| laSexta", "1", "lasexta.es", false], ["DEP| Deportes Uno", "2", "dep1.es", false], ["DEP| Deportes Dos 4K", "2", "dep2.es", false],
  ["DEP| Fútbol Total", "2", "futbol.es", true], ["NOT| 24 Horas", "3", "24h.es", false], ["NOT| Noticias Mundo", "3", "mundo.es", false],
  ["NOT| Canal Economía", "3", null, false], ["INF| Dibus", "4", "dibus.es", false], ["INF| Clan", "4", "clan.es", false],
  ["XXX| Canal Adulto", "9", null, false], ["ES| Reloj (pruebas de pausa)", "1", null, false],
];
const channels = channelSpecs.map(([name, category, epg, catchup], i) => ({
  num: i + 1,
  name,
  stream_type: "live",
  stream_id: 100 + i,
  stream_icon: `${BASE}/logos/${100 + i}.svg`,
  epg_channel_id: epg,
  added: String(now - 30 * DAY),
  category_id: category,
  tv_archive: catchup ? 1 : 0,
  tv_archive_duration: catchup ? "3" : "0",
}));

const vodCategories = [
  { category_id: "10", category_name: "ESTRENOS", parent_id: 0 },
  { category_id: "11", category_name: "CLÁSICOS", parent_id: 0 },
  { category_id: "12", category_name: "INFANTIL", parent_id: 0 },
  { category_id: "19", category_name: "XXX ADULTOS", parent_id: 0 },
];

// Títulos de dominio público o muy conocidos (si se añade clave de TMDB, emparejan), con
// la suciedad habitual: año entre paréntesis o corchetes, calidad, idioma, duplicados
// en varias calidades que deben agruparse en una sola ficha.
const movieSpecs = [
  ["ES| Nosferatu (1922)", "11", "mp4"], ["Metropolis [1927] FHD", "11", "mkv"], ["Metropolis (1927) 4K", "11", "mp4"],
  ["El maquinista de la General (1926)", "11", "mp4"], ["La noche de los muertos vivientes (1968) HD", "11", "mkv"],
  ["Charada (1963)", "11", "mp4"], ["Luna nueva (1940)", "11", "mp4"], ["El chico (1921)", "11", "mkv"],
  ["Sherlock Jr. (1924)", "11", "mp4"], ["El gabinete del doctor Caligari (1920)", "11", "mp4"],
  ["ES| Matrix (1999) 1080p", "10", "mkv"], ["Gladiator [2000] FHD", "10", "mp4"], ["Origen (2010) 4K HDR", "10", "mkv"],
  ["Interstellar (2014)", "10", "mp4"], ["Blade Runner 2049 (2017) 4K", "10", "mkv"], ["1917 (2019)", "10", "mp4"],
  ["Dune (2021) MULTI", "10", "mkv"], ["Oppenheimer (2023) CASTELLANO", "10", "mp4"],
  ["Planeta Tierra II (2016) 4K HEVC", "10", "mkv"], ["Akira (1988)", "12", "mkv"],
  ["Toy Story (1995)", "12", "mp4"], ["Buscando a Nemo (2003)", "12", "mp4"], ["Up (2009) HD", "12", "mkv"],
  ["Del revés (2015)", "12", "mp4"], ["XXX| Película Adulta (2020)", "19", "mp4"],
];
const movies = movieSpecs.map(([name, category, ext], i) => ({
  num: i + 1,
  name,
  stream_type: "movie",
  stream_id: 5000 + i,
  stream_icon: `${BASE}/covers/${5000 + i}.svg`,
  // Los cinco primeros de estrenos son "nuevos" (insignia de los últimos 7 días).
  added: String(category === "10" && i < 15 ? now - 2 * DAY : now - 90 * DAY),
  rating: (5 + (i % 5)).toFixed(1),
  container_extension: ext,
  category_id: category,
}));

const seriesCategories = [
  { category_id: "20", category_name: "SERIES ESPAÑOLAS", parent_id: 0 },
  { category_id: "21", category_name: "SERIES INTERNACIONALES", parent_id: 0 },
];
const seriesSpecs = [
  ["ES| La Casa de Papel", "20", "Drama", "2017-05-02", [3, 2]],
  ["Breaking Bad (2008)", "21", "Drama", "2008-01-20", [3, 3]],
  ["Stranger Things 4K", "21", "Ciencia ficción", "2016-07-15", [2, 2]],
  ["ES| Aquí no hay quien viva", "20", "Comedia", "2003-09-07", [4, 2]],
];
const series = seriesSpecs.map(([name, category, genre, release, _seasons], i) => ({
  num: i + 1,
  name,
  series_id: 9000 + i,
  cover: `${BASE}/covers/${9000 + i}.svg`,
  plot: `Serie de prueba número ${i + 1} del proveedor falso.`,
  genre,
  releaseDate: release,
  last_modified: String(now - 10 * DAY),
  category_id: category,
}));

function seriesInfo(seriesId) {
  const index = seriesId - 9000;
  const spec = seriesSpecs[index];
  if (!spec) return { seasons: [], info: {}, episodes: {} };
  const [episodesPerSeason, seasonCount] = spec[4];
  const episodes = {};
  const seasons = [];
  for (let s = 1; s <= seasonCount; s++) {
    seasons.push({ season_number: s, name: `Temporada ${s}` });
    episodes[String(s)] = Array.from({ length: episodesPerSeason }, (_, e) => ({
      id: String(90000 + index * 100 + s * 10 + e + 1),
      episode_num: e + 1,
      title: `${spec[0].replace(/^ES\| /, "")} S${String(s).padStart(2, "0")}E${String(e + 1).padStart(2, "0")}`,
      container_extension: e % 2 === 0 ? "mp4" : "mkv",
      info: { duration_secs: "180", movie_image: `${BASE}/covers/${seriesId}.svg` },
    }));
  }
  return { seasons, info: { name: spec[0], cover: `${BASE}/covers/${seriesId}.svg`, plot: series[index].plot }, episodes };
}

// ─────────────── EPG (XMLTV) ───────────────

const programmeTitles = ["Telediario", "Cine de tarde", "Documental: La Tierra", "Magazine matinal", "Concurso", "Deportes en directo", "Serie del día", "Noticias de la noche", "Dibujos animados", "Debate"];

function xmltvTime(epoch) {
  const d = new Date(epoch * 1000);
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getUTCFullYear()}${p(d.getUTCMonth() + 1)}${p(d.getUTCDate())}${p(d.getUTCHours())}${p(d.getUTCMinutes())}${p(d.getUTCSeconds())} +0000`;
}

function escapeXml(text) {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function buildXmltv() {
  const epgIds = [...new Set(channels.map((c) => c.epg_channel_id).filter(Boolean))];
  const start = Math.floor((now - DAY) / 3600) * 3600;
  const end = now + 2 * DAY;
  let xml = '<?xml version="1.0" encoding="UTF-8"?>\n<tv generator-info-name="proveedor-falso">\n';
  for (const id of epgIds) xml += `  <channel id="${id}"><display-name>${id}</display-name></channel>\n`;
  epgIds.forEach((id, channelIndex) => {
    let t = start;
    let n = channelIndex;
    while (t < end) {
      const length = (n % 3 === 0 ? 60 : 30) * 60;
      const title = programmeTitles[n % programmeTitles.length];
      xml += `  <programme start="${xmltvTime(t)}" stop="${xmltvTime(t + length)}" channel="${id}"><title>${escapeXml(title)}</title><desc>Programa de prueba en ${id}.</desc></programme>\n`;
      t += length;
      n++;
    }
  });
  return xml + "</tv>\n";
}

// ─────────────── Imágenes ───────────────

function svgCard(label, hue, wide) {
  const [w, h] = wide ? [400, 225] : [300, 450];
  const text = escapeXml(label.replace(/^[A-Z]+\| /, "").slice(0, 22));
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}"><rect width="100%" height="100%" fill="hsl(${hue},45%,30%)"/><text x="50%" y="50%" fill="#fff" font-family="sans-serif" font-size="${wide ? 34 : 28}" text-anchor="middle" dominant-baseline="middle">${text}</text></svg>`;
}

// ─────────────── Servidor ───────────────

function sendJson(res, value) {
  const body = JSON.stringify(value);
  res.writeHead(200, { "content-type": "application/json; charset=utf-8", "content-length": Buffer.byteLength(body) });
  res.end(body);
}

/** Fichero con soporte de Range, como sirve un panel real las películas. */
function sendFile(req, res, path, contentType) {
  const size = statSync(path).size;
  const range = /bytes=(\d*)-(\d*)/.exec(req.headers.range ?? "");
  if (range) {
    const start = range[1] ? Number(range[1]) : 0;
    const end = range[2] ? Math.min(Number(range[2]), size - 1) : size - 1;
    res.writeHead(206, { "content-type": contentType, "accept-ranges": "bytes", "content-range": `bytes ${start}-${end}/${size}`, "content-length": end - start + 1 });
    createReadStream(path, { start, end }).pipe(res);
    return;
  }
  res.writeHead(200, { "content-type": contentType, "accept-ranges": "bytes", "content-length": size });
  createReadStream(path).pipe(res);
}

/** Directo: el TS se envía al ritmo de reproducción, sin fin conocido, como un canal. */
let openLiveConnections = 0;
// Cuándo empezó el solapamiento actual (dos conexiones a la vez), para medir cuánto duró:
// unos milisegundos son el servidor enterándose tarde del cierre del socket anterior; más
// es una segunda conexión de verdad (regla R1).
let overlapStartedAt = null;
function sendLive(req, res, spec) {
  const path = join(MEDIA_DIR, spec.file);
  const size = statSync(path).size;
  const bytesPerTick = Math.ceil((size / spec.seconds) / 10);
  // Arranque rápido: los primeros segundos de golpe, como el búfer inicial de un panel.
  let offset = 0;
  const fd = openSync(path, "r");
  openLiveConnections++;
  if (openLiveConnections > 1 && overlapStartedAt === null) overlapStartedAt = Date.now();
  console.log(`directo abierto (${openLiveConnections} a la vez)${openLiveConnections > 1 ? "  ← más de una conexión, midiendo…" : ""}`);
  res.writeHead(200, { "content-type": "video/mp2t" });
  const pushBytes = (count) => {
    const buffer = Buffer.alloc(Math.min(count, size - offset));
    const read = readSync(fd, buffer, 0, buffer.length, offset);
    offset += read;
    return res.write(buffer.subarray(0, read));
  };
  pushBytes(bytesPerTick * 30);
  const timer = setInterval(() => {
    if (offset >= size) {
      clearInterval(timer);
      res.end();
      return;
    }
    pushBytes(bytesPerTick);
  }, 100);
  const close = () => {
    clearInterval(timer);
    closeSync(fd);
    openLiveConnections--;
    if (openLiveConnections <= 1 && overlapStartedAt !== null) {
      const ms = Date.now() - overlapStartedAt;
      overlapStartedAt = null;
      console.log(`solapamiento de ${ms} ms${ms > 250 ? "  ← ¡DOS CONEXIONES DE VERDAD! (R1)" : " (cierre detectado tarde)"}`);
    }
    console.log(`directo cerrado (${openLiveConnections} a la vez)`);
  };
  res.once("close", close);
}

function playerApi(url, res) {
  const params = url.searchParams;
  if (params.get("username") !== USER || params.get("password") !== PASS) {
    return sendJson(res, { user_info: { auth: 0 } });
  }
  const action = params.get("action");
  const category = params.get("category_id");
  const byCategory = (list) => (category ? list.filter((item) => item.category_id === category) : list);
  switch (action) {
    case null:
      return sendJson(res, {
        user_info: { username: USER, password: PASS, auth: 1, status: "Active", exp_date: String(now + 45 * DAY), is_trial: "0", active_cons: String(openLiveConnections), max_connections: "1", allowed_output_formats: ["ts", "m3u8"] },
        server_info: { url: "127.0.0.1", port: String(PORT), server_protocol: "http", timezone: "Europe/Madrid", timestamp_now: now },
      });
    case "get_live_categories": return sendJson(res, liveCategories);
    case "get_vod_categories": return sendJson(res, vodCategories);
    case "get_series_categories": return sendJson(res, seriesCategories);
    case "get_live_streams": return sendJson(res, byCategory(channels));
    case "get_vod_streams": return sendJson(res, byCategory(movies));
    case "get_series": return sendJson(res, byCategory(series));
    case "get_series_info": return sendJson(res, seriesInfo(Number(params.get("series_id"))));
    case "get_vod_info": {
      const movie = movies.find((m) => String(m.stream_id) === params.get("vod_id"));
      const spec = movie?.container_extension === "mkv" ? MEDIA.mkv : MEDIA.mp4;
      return sendJson(res, {
        info: { duration_secs: spec.seconds, duration: "00:03:00", video: { codec_name: "h264", width: 640, height: 360 }, audio: { codec_name: "aac" } },
        movie_data: { stream_id: movie?.stream_id ?? 0, container_extension: movie?.container_extension ?? "mp4" },
      });
    }
    default:
      return sendJson(res, []);
  }
}

ensureMedia();
const xmltv = gzipSync(buildXmltv());

createServer((req, res) => {
  const url = new URL(req.url ?? "/", BASE);
  const path = url.pathname;
  const credentials = `/${USER}/${PASS}/`;
  console.log(`${req.method} ${path}${url.search ? " ?" + (url.searchParams.get("action") ?? "") : ""}`);

  if (path === "/player_api.php") return playerApi(url, res);
  if (path === "/xmltv.php") {
    // Gzip sin declarar Content-Encoding: lo hacen muchos paneles reales.
    res.writeHead(200, { "content-type": "application/xml" });
    return res.end(xmltv);
  }
  const logo = /^\/(logos|covers)\/(\d+)\.svg$/.exec(path);
  if (logo) {
    const id = Number(logo[2]);
    const item = channels.find((c) => c.stream_id === id) ?? movies.find((m) => m.stream_id === id) ?? series.find((s) => s.series_id === id);
    res.writeHead(200, { "content-type": "image/svg+xml" });
    return res.end(svgCard(item?.name ?? String(id), (id * 47) % 360, logo[1] === "logos"));
  }
  if (path.startsWith(`/live${credentials}`) || path.startsWith("/timeshift/") || path.startsWith("/streaming/timeshift.php")) {
    const id = Number(/(\d+)\.ts$/.exec(path)?.[1] ?? url.searchParams.get("stream"));
    // Los canales de deportes llevan audio AC3 (el remux tiene que recodificarlo a AAC).
    const channel = channels.find((c) => c.stream_id === id);
    if (channel && /Reloj/.test(channel.name)) return sendLive(req, res, MEDIA.liveClock);
    return sendLive(req, res, channel?.category_id === "2" ? MEDIA.liveAc3 : MEDIA.live);
  }
  const vod = new RegExp(`^/(movie|series)${credentials}(\\d+)\\.(mp4|mkv)$`).exec(path);
  if (vod) {
    const isMkv = vod[3] === "mkv";
    const movie = movies.find((m) => String(m.stream_id) === vod[2]);
    if (movie && /HEVC/.test(movie.name)) return sendFile(req, res, join(MEDIA_DIR, MEDIA.hevc.file), "video/x-matroska");
    if (movie && /^Akira/.test(movie.name)) return sendFile(req, res, join(MEDIA_DIR, MEDIA.hi10.file), "video/x-matroska");
    return sendFile(req, res, join(MEDIA_DIR, isMkv ? MEDIA.mkv.file : MEDIA.mp4.file), isMkv ? "video/x-matroska" : "video/mp4");
  }
  res.writeHead(404);
  res.end();
}).listen(PORT, "127.0.0.1", () => {
  console.log(`Proveedor falso en ${BASE}  (usuario "${USER}", contraseña "${PASS}")`);
  console.log(`${channels.length} canales, ${movies.length} películas, ${series.length} series`);
});
