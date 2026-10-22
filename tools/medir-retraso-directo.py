"""Mide cuánto retraso lleva cada fuente de un canal en directo respecto a la emisión real.

Muchas emisiones de TV llevan dentro la hora de la emisión original (tabla TDT/TOT del
MPEG-TS, PID 0x14, la que usan los decodificadores para poner el reloj en hora). Si una
fuente la conserva, retraso = hora a la que llega - hora de emisión que trae. Con eso se
puede saber qué fuente de un partido va por delante.

Habla solo con la app de escritorio ya abierta (su API local en 127.0.0.1): nunca ve la
URL ni las credenciales del proveedor (regla R5) y cada fuente pasa por el StreamGuard de
la app, de una en una y con pausa entre ellas (regla R1: la cuenta admite una conexión).

Uso:  python tools/medir-retraso-directo.py <puerto> "<nombre del canal>" [segundos]
"""

import json
import sys
import time
import urllib.parse
import urllib.request
from datetime import datetime, timedelta, timezone

TS_PACKET = 188
TDT_PID = 0x14
PAUSE_BETWEEN_SOURCES_S = 4


def api(base, path, method="GET"):
    request = urllib.request.Request(base + path, method=method)
    with urllib.request.urlopen(request, timeout=15) as response:
        body = response.read()
        return json.loads(body) if body else None


def mjd_bcd_to_datetime(data):
    """Fecha y hora UTC de la TDT/TOT: 16 bits de día juliano modificado y hh mm ss en BCD."""
    mjd = (data[0] << 8) | data[1]
    year = int((mjd - 15078.2) / 365.25)
    month = int((mjd - 14956.1 - int(year * 365.25)) / 30.6001)
    day = mjd - 14956 - int(year * 365.25) - int(month * 30.6001)
    k = 1 if month in (14, 15) else 0
    year, month = year + k + 1900, month - 1 - k * 12

    def bcd(byte):
        return (byte >> 4) * 10 + (byte & 0x0F)

    return datetime(year, month, day, tzinfo=timezone.utc) + timedelta(hours=bcd(data[2]), minutes=bcd(data[3]), seconds=bcd(data[4]))


def measure(base, profile_id, channel_id, seconds):
    """Lee la fuente unos segundos y devuelve los retrasos medidos (uno por cada TDT/TOT)."""
    url = f"{base}/stream/live-buffer/channel/{channel_id}?profile_id={profile_id}"
    delays, buffer, started = [], b"", time.time()
    try:
        with urllib.request.urlopen(url, timeout=20) as response:
            while time.time() - started < seconds:
                chunk = response.read(TS_PACKET * 64)
                if not chunk:
                    break
                arrived = datetime.now(timezone.utc)
                buffer += chunk
                while len(buffer) >= TS_PACKET:
                    if buffer[0] != 0x47:
                        sync = buffer.find(b"\x47", 1)
                        buffer = buffer[sync:] if sync > 0 else b""
                        continue
                    packet, buffer = buffer[:TS_PACKET], buffer[TS_PACKET:]
                    pid = ((packet[1] & 0x1F) << 8) | packet[2]
                    if pid != TDT_PID or not packet[1] & 0x40:
                        continue
                    payload = 4
                    if packet[3] & 0x20:  # campo de adaptación
                        payload += 1 + packet[4]
                    payload += 1 + packet[payload]  # pointer_field
                    table_id = packet[payload]
                    if table_id in (0x70, 0x73):
                        try:
                            emitted = mjd_bcd_to_datetime(packet[payload + 3 : payload + 8])
                        except ValueError:
                            continue
                        delays.append((arrived - emitted).total_seconds())
    finally:
        # Suelta ya la conexión con el proveedor.
        try:
            api(base, f"/api/live-buffer?channel_id={channel_id}", method="DELETE")
        except Exception:
            pass
    return delays


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(1)
    base = f"http://127.0.0.1:{int(sys.argv[1])}"
    name = sys.argv[2]
    seconds = int(sys.argv[3]) if len(sys.argv) > 3 else 12

    profiles = api(base, "/api/profiles") or []
    profile_id = profiles[0]["id"] if profiles else 1
    page = api(base, f"/api/live/channels?profile_id={profile_id}&limit=50&q={urllib.parse.quote(name)}")
    sources = page["items"] if page else []
    # Solo las fuentes de ese canal, no las de nombre parecido («DAZN LaLiga 2»).
    sources = [x for x in sources if x["display_name"].strip().lower() == name.strip().lower()] or sources
    if not sources:
        print(f"No hay ningún canal que se llame «{name}».")
        return
    print(f"{len(sources)} fuente(s) de «{name}». Se mide cada una {seconds} s, de una en una.\n")
    for index, source in enumerate(sources[:6]):
        if index:
            time.sleep(PAUSE_BETWEEN_SOURCES_S)
        label = f"{source['display_name']} (#{source['id']})"
        try:
            delays = measure(base, profile_id, source["id"], seconds)
        except Exception as error:  # sin URL: el error de urllib solo lleva la local
            print(f"  {label}: no se pudo abrir ({type(error).__name__})")
            continue
        if not delays:
            print(f"  {label}: no trae la hora de emisión (no se puede medir)")
            continue
        # Lo primero que llega es el búfer inicial del proveedor (más viejo): manda lo último.
        steady = delays[len(delays) // 2 :]
        print(f"  {label}: {min(steady):.0f} s de retraso ({len(delays)} marcas de hora)")


if __name__ == "__main__":
    main()
