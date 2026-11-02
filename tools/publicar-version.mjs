// Publica una versión de la app de escritorio para Windows en GitHub Releases, de donde la
// sacan las actualizaciones automáticas (apps/desktop/src-tauri/src/updater.rs).
//
//   node tools/publicar-version.mjs 0.2.0 [--notas fichero.md] [--sin-publicar]
//
// Necesita: los sidecars en apps/desktop/src-tauri/binaries (ver su LEEME.md), `gh` con
// sesión iniciada y la clave de firma en ~/.tauri/vibe-iptv.key (o en la variable
// VIBE_SIGNING_KEY_PATH). La clave NO está en el repositorio: si se pierde, las copias
// instaladas ya no aceptarán versiones nuevas (habría que reinstalar a mano).
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const REPO = "Valmaseda10/vibe-iptv";
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const desktop = join(root, "apps", "desktop");
const tauriDir = join(desktop, "src-tauri");

const args = process.argv.slice(2);
const version = args[0];
if (!version || !/^\d+\.\d+\.\d+$/.test(version)) {
  console.error("Uso: node tools/publicar-version.mjs X.Y.Z [--notas fichero.md] [--sin-publicar]");
  process.exit(1);
}
const notesIndex = args.indexOf("--notas");
const notesFile = notesIndex >= 0 ? args[notesIndex + 1] : null;
const publish = !args.includes("--sin-publicar");

const keyPath = process.env.VIBE_SIGNING_KEY_PATH ?? join(homedir(), ".tauri", "vibe-iptv.key");
if (!existsSync(keyPath)) {
  console.error(`No encuentro la clave de firma en ${keyPath}.`);
  process.exit(1);
}
for (const bin of ["ffmpeg", "ffprobe", "mpv"]) {
  if (!existsSync(join(tauriDir, "binaries", `${bin}-x86_64-pc-windows-msvc.exe`))) {
    console.error(`Falta el sidecar ${bin} en src-tauri/binaries (ver LEEME.md).`);
    process.exit(1);
  }
}

// 1. La versión, en los tres sitios donde vive.
function bump(file, pattern, replacement) {
  const text = readFileSync(file, "utf8");
  if (!pattern.test(text)) throw new Error(`No encuentro la versión en ${file}`);
  writeFileSync(file, text.replace(pattern, replacement));
}
bump(join(tauriDir, "tauri.conf.json"), /"version": "[^"]+"/, `"version": "${version}"`);
bump(join(desktop, "package.json"), /"version": "[^"]+"/, `"version": "${version}"`);
bump(join(tauriDir, "Cargo.toml"), /^version = "[^"]+"/m, `version = "${version}"`);

// Las novedades van dentro de la app («Novedades» al abrir la versión nueva, y en Ajustes →
// Acerca de), también en la de iPhone, que se compila desde el repositorio.
if (notesFile) {
  const notes = readFileSync(notesFile, "utf8")
    .split(/?
/)
    .filter((line) => line.startsWith("- "))
    .map((line) => line.slice(2).trim());
  writeFileSync(
    join(desktop, "src", "lib", "releaseNotes.generated.ts"),
    `// Generado por tools/publicar-version.mjs a partir de las notas de la versión: no se edita a mano.
export const RELEASE_NOTES: { version: string; notes: readonly string[] } = ${JSON.stringify({ version, notes }, null, 2)};
`,
  );
}

// 2. Instalador firmado (el actualizador comprueba la firma con la clave pública de tauri.conf.json).
console.log(`Compilando la versión ${version}…`);
execFileSync("npm", ["run", "build:installer"], {
  cwd: desktop,
  stdio: "inherit",
  shell: true,
  env: {
    ...process.env,
    TAURI_SIGNING_PRIVATE_KEY: readFileSync(keyPath, "utf8"),
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD: "",
    // Sin rutas de este PC dentro del programa (Rust guarda las de las fuentes para sus
    // mensajes de error): llevaban el nombre de usuario de Windows.
    RUSTFLAGS: `--remap-path-prefix=${homedir()}=~ --remap-path-prefix=${root}=vibe-iptv`,
  },
});

// Antes de publicar nada: ni rutas de este PC ni la clave de firma dentro del programa.
const exe = readFileSync(join(tauriDir, "target", "release", "vibe-iptv.exe"));
const forbidden = [homedir(), readFileSync(keyPath, "utf8").trim().slice(0, 40)];
for (const text of forbidden) {
  if (exe.includes(Buffer.from(text))) throw new Error("El programa contiene datos de este PC (ruta de usuario o clave): no se publica.");
}

const bundleDir = join(tauriDir, "target", "release", "bundle", "nsis");
const built = join(bundleDir, `VIBE IPTV_${version}_x64-setup.exe`);
if (!existsSync(built) || !existsSync(`${built}.sig`)) throw new Error(`No se generó ${built} con su firma`);

// 3. Sin espacios en el nombre: GitHub los cambia por puntos y la URL ya no coincidiría.
const outDir = join(root, "target-release", version);
mkdirSync(outDir, { recursive: true });
const assetName = `VIBE-IPTV_${version}_x64-setup.exe`;
const asset = join(outDir, assetName);
copyFileSync(built, asset);
const latest = {
  version,
  notes: notesFile ? readFileSync(notesFile, "utf8").trim() : `VIBE IPTV ${version}`,
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      signature: readFileSync(`${built}.sig`, "utf8").trim(),
      url: `https://github.com/${REPO}/releases/download/v${version}/${assetName}`,
    },
  },
};
const latestPath = join(outDir, "latest.json");
writeFileSync(latestPath, JSON.stringify(latest, null, 2));

const releaseNotes = join(outDir, "notas.md");
writeFileSync(
  releaseNotes,
  `${notesFile ? readFileSync(notesFile, "utf8").trim() : `VIBE IPTV ${version}`}

---

Instalador para Windows 10/11 (64 bits). Las copias ya instaladas se actualizan solas.
La app no incluye contenido ni listas: se conecta a tu propio proveedor IPTV.

Incluye ffmpeg y mpv como programas aparte (GPL). Su código fuente: ffmpeg en
https://www.gyan.dev/ffmpeg/builds/ (y https://github.com/FFmpeg/FFmpeg), mpv en
https://github.com/shinchiro/mpv-winbuild-cmake (y https://github.com/mpv-player/mpv).
`,
);

console.log(`Listo en ${outDir}`);
if (!publish) {
  console.log("--sin-publicar: no se sube nada a GitHub.");
  process.exit(0);
}

// 4. La release. `latest.json` de la última es lo que miran las copias instaladas.
execFileSync("gh", ["release", "create", `v${version}`, asset, latestPath, "--repo", REPO, "--title", `VIBE IPTV ${version}`, "--notes-file", releaseNotes], {
  stdio: "inherit",
});
console.log(`Publicada: https://github.com/${REPO}/releases/tag/v${version}`);
