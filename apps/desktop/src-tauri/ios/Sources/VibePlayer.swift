// Reproductor nativo de iPhone y iPad (F15): VLC a pantalla completa por encima de la
// interfaz web. Ver `src/native_player.rs` para el puente con el núcleo.
//
// Lo que entra: `vibe_player_present(json)`, llamado desde Rust.
// Lo que sale: `vibe_player_event(json)`, implementado en Rust, que lo reenvía a la
// interfaz. Los eventos son los de `NativePlayerEvent` en `native_player.rs`.
//
// Gestos (decididos con el usuario):
// - Un toque: enseñar o esconder los controles.
// - Doble toque: a la izquierda −10 s, a la derecha +10 s, en el centro llenar la pantalla.
// - Pellizcar: llenar la pantalla o ver la imagen entera.
// - Deslizar en horizontal: avanzar o retroceder (en directo grabado, moverse por lo grabado).
// - Deslizar en vertical: borde izquierdo brillo, borde derecho volumen; en el centro, en
//   directo cambia de canal y en películas hacia abajo cierra.

import AVFoundation
import MediaPlayer
import MobileVLCKit
import UIKit

@_silgen_name("vibe_player_event")
private func vibePlayerEventToRust(_ json: UnsafePointer<CChar>)

private func sendEvent(_ payload: [String: Any]) {
    guard let data = try? JSONSerialization.data(withJSONObject: payload),
          let text = String(data: data, encoding: .utf8) else { return }
    text.withCString { vibePlayerEventToRust($0) }
}

private struct PlayItem: Decodable {
    let id: Int64
    let title: String
    let subtitle: String?
    let number: Int64?
    let url: String
}

private struct PlayRequest: Decodable {
    let url: String
    let title: String
    let subtitle: String?
    let start_seconds: Double
    let live: Bool
    /// El directo sale de la grabación del núcleo: se puede pausar y moverse por ella
    /// pidiendo la misma URL con `&behind_s=N` (ver `server::live_buffer`).
    let live_buffer: Bool?
    /// El motivo real de un fallo, para enseñarlo en el reproductor ya abierto.
    let message: String?
    let kind: String?
    let items: [PlayItem]?
    let current_index: Int?
    let next: PlayItem?
    let preferred_audio: String?
    let preferred_subtitle: String?
    /// Solo actualiza la lista, el siguiente episodio y las preferencias del reproductor ya
    /// abierto, sin volver a cargar el vídeo.
    let update_only: Bool?
}

/// Lo que se está reproduciendo ahora (puede cambiar sin pasar por la interfaz web).
private struct Current {
    var id: Int64?
    var url: String
    var title: String
    var subtitle: String?
    var live: Bool
    var liveBuffer: Bool
}

@_cdecl("vibe_player_present")
public func vibePlayerPresent(_ json: UnsafePointer<CChar>) {
    // Se copia antes de volver: el puntero solo vale durante la llamada.
    let text = String(cString: json)
    DispatchQueue.main.async {
        guard let data = text.data(using: .utf8),
              let request = try? JSONDecoder().decode(PlayRequest.self, from: data) else { return }
        VibePlayerController.show(request)
    }
}

/// Raíz de la ventana de los controles (ver `VibePlayerController.controlsWindow`): misma
/// orientación que el reproductor y transparente, para que se vea el vídeo de debajo.
private final class ControlsHostController: UIViewController {
    override var prefersStatusBarHidden: Bool { true }
    override var prefersHomeIndicatorAutoHidden: Bool { true }
    override var supportedInterfaceOrientations: UIInterfaceOrientationMask {
        UIDevice.current.userInterfaceIdiom == .pad ? .all : .landscape
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .clear
    }
}

/// Fondo de los controles: oscuro arriba y abajo, transparente en el centro.
private final class GradientView: UIView {
    override class var layerClass: AnyClass { CAGradientLayer.self }

    override init(frame: CGRect) {
        super.init(frame: frame)
        let gradient = layer as! CAGradientLayer
        gradient.colors = [
            UIColor.black.withAlphaComponent(0.75).cgColor,
            UIColor.black.withAlphaComponent(0.15).cgColor,
            UIColor.black.withAlphaComponent(0.15).cgColor,
            UIColor.black.withAlphaComponent(0.8).cgColor,
        ]
        gradient.locations = [0, 0.3, 0.65, 1]
    }

    required init?(coder: NSCoder) { fatalError("no se usa desde storyboard") }
}

final class VibePlayerController: UIViewController {
    private static weak var current: VibePlayerController?

    /// Abre el reproductor, o cambia lo que reproduce si ya está abierto (zapping).
    fileprivate static func show(_ request: PlayRequest) {
        if let controller = current, controller.presentingViewController != nil {
            if let message = request.message {
                controller.spinner.stopAnimating()
                controller.showMessage(message)
            } else if request.update_only == true {
                controller.update(request)
            } else {
                controller.load(request)
            }
            return
        }
        if request.message != nil || request.update_only == true { return }
        guard let presenter = topViewController() else { return }
        let controller = VibePlayerController()
        controller.modalPresentationStyle = .fullScreen
        current = controller
        presenter.present(controller, animated: true) { controller.load(request) }
    }

    private static func topViewController() -> UIViewController? {
        let window = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap { $0.windows }
            .first { $0.isKeyWindow }
        var top = window?.rootViewController
        while let presented = top?.presentedViewController { top = presented }
        return top
    }

    // `--http-reconnect`: un directo IPTV se corta a menudo un instante; VLC reabre solo.
    //
    // Uno solo para toda la vida de la app, nunca liberado: creado con opciones, VLC le da
    // su propia instancia de libVLC y la liberaba al cerrar el reproductor; si uno de sus
    // hilos de red seguía vivo, escribía en el registro de esa instancia ya liberada y la
    // app se cerraba (EXC_BAD_ACCESS en `vlc_http_dbg`, visto en el simulador al cerrar un
    // directo).
    private static let sharedPlayer = VLCMediaPlayer(options: ["--http-reconnect"])
    private let player = VibePlayerController.sharedPlayer
    /// Sube cada vez que el reproductor empieza algo: un aviso de cierre pendiente sabe así
    /// si entretanto se abrió otra cosa.
    private static var playGeneration = 0
    private var request: PlayRequest?
    private var current: Current?
    private var items: [PlayItem] = []
    private var currentIndex: Int?
    private var nextItem: PlayItem?
    private var closed = false

    private let videoView = UIView()
    /// Los controles van en una ventana propia por encima de la del vídeo. VLC pinta su
    /// vista de vídeo desde su propio hilo y acaba encima de cualquier vista hermana: los
    /// controles existían, recibían los toques, y no se veían (comprobado en el simulador
    /// volcando la jerarquía de vistas). Una ventana de nivel superior siempre se compone
    /// por encima.
    private let controlsHost = ControlsHostController()
    private var controlsWindow: UIWindow?
    private var controlsRoot: UIView { controlsHost.view }
    private let overlay = GradientView()
    private let titleLabel = UILabel()
    private let subtitleLabel = UILabel()
    private let closeButton = UIButton(type: .system)
    private let listButton = UIButton(type: .system)
    private let audioButton = UIButton(type: .system)
    private let subtitlesButton = UIButton(type: .system)
    private let moreButton = UIButton(type: .system)
    private let playButton = UIButton(type: .system)
    private let backButton = UIButton(type: .system)
    private let forwardButton = UIButton(type: .system)
    private let slider = UISlider()
    private let timeLabel = UILabel()
    private let durationLabel = UILabel()
    private let liveBadge = UILabel()
    private let goLiveButton = UIButton(type: .system)
    private let aspectButton = UIButton(type: .system)
    private let spinner = UIActivityIndicatorView(style: .large)
    private let messageLabel = UILabel()
    private let hudLabel = PaddedLabel()
    private let skipIntroButton = UIButton(type: .system)
    private let nextCard = UIView()
    private let nextTitleLabel = UILabel()
    private let nextPlayButton = UIButton(type: .system)
    private let nextCancelButton = UIButton(type: .system)
    private let lockButton = UIButton(type: .system)
    private let listPanel = UIView()
    private let listTable = UITableView(frame: .zero, style: .plain)
    private let volumeView = MPVolumeView(frame: CGRect(x: -2000, y: -2000, width: 10, height: 10))

    private var tickTimer: Timer?
    private var hideTimer: Timer?
    private var hudTimer: Timer?
    private var sleepTimer: Timer?
    private var sleepAtEnd = false
    private var nextCountdown: Timer?
    private var nextSecondsLeft = 0
    private var nextDismissed = false
    private var lastProgressSent = Date.distantPast
    private var lastNowPlaying = Date.distantPast
    private var scrubbing = false
    private var locked = false
    /// Directo grabado: segundos por detrás del directo de lo que se ve, y desde cuándo
    /// está en pausa (la pausa también suma retraso).
    private var behindLive: Double = 0
    private var pausedAt: Date?
    private var liveReconnects = 0
    /// Última posición conocida en VOD y reaperturas tras un corte a mitad.
    private var lastPosition: Double = 0
    private var vodReconnects = 0
    private var errorRetries = 0
    private var lastRetryAt = Date.distantPast
    /// Los controles se enseñan al empezar a verse cada vídeo (también tras un zapping).
    private var shownForThisMedia = false
    private var tracksApplied = false
    /// Rellenar la pantalla recortando los bordes, en vez de ver la imagen entera.
    private var fillScreen = false
    private var savedVideoTrack: Int32?
    private var panMode: PanMode = .none
    /// Cristal de la barra de abajo (tiempos y progreso): en directo no hay nada que poner.
    private var bottomGlass: UIView?
    private var panStart: CGFloat = 0
    private var panStartValue: Double = 0
    private let symbolConfig = UIImage.SymbolConfiguration(pointSize: 21, weight: .semibold)
    private let bigSymbolConfig = UIImage.SymbolConfiguration(pointSize: 40, weight: .semibold)

    private enum PanMode { case none, seek, brightness, volume, zap }

    private var isLive: Bool { current?.live == true }
    private var isLiveBuffer: Bool { current?.live == true && current?.liveBuffer == true }

    override var prefersStatusBarHidden: Bool { true }
    override var prefersHomeIndicatorAutoHidden: Bool { true }
    override var supportedInterfaceOrientations: UIInterfaceOrientationMask {
        UIDevice.current.userInterfaceIdiom == .pad ? .all : .landscape
    }
    override var preferredInterfaceOrientationForPresentation: UIInterfaceOrientation { .landscapeRight }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        try? AVAudioSession.sharedInstance().setCategory(.playback, mode: .moviePlayback)
        try? AVAudioSession.sharedInstance().setActive(true)

        videoView.backgroundColor = .black
        videoView.translatesAutoresizingMaskIntoConstraints = false
        videoView.isUserInteractionEnabled = false
        view.addSubview(videoView)
        player.drawable = videoView
        // El reproductor se reutiliza: lo de la vez anterior no se arrastra.
        player.rate = 1
        NSLayoutConstraint.activate([
            videoView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            videoView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            videoView.topAnchor.constraint(equalTo: view.topAnchor),
            videoView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])

        buildOverlay()
        buildGestures()
        setupRemoteCommands()

        let center = NotificationCenter.default
        center.addObserver(self, selector: #selector(didEnterBackground), name: UIApplication.didEnterBackgroundNotification, object: nil)
        center.addObserver(self, selector: #selector(willEnterForeground), name: UIApplication.willEnterForegroundNotification, object: nil)
        tickTimer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in self?.tick() }
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        UIApplication.shared.isIdleTimerDisabled = true
        if controlsWindow == nil, let scene = view.window?.windowScene {
            let window = UIWindow(windowScene: scene)
            window.windowLevel = .normal + 1
            window.backgroundColor = .clear
            window.rootViewController = controlsHost
            window.isHidden = false
            controlsWindow = window
            showOverlay()
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        UIApplication.shared.isIdleTimerDisabled = false
        removeControlsWindow()
    }

    private func removeControlsWindow() {
        controlsWindow?.isHidden = true
        controlsWindow?.rootViewController = nil
        controlsWindow = nil
    }

    // MARK: - Reproducción

    private func load(_ request: PlayRequest) {
        self.request = request
        update(request)
        let item = Current(
            id: request.current_index.flatMap { $0 < items.count ? items[$0].id : nil },
            url: request.url, title: request.title, subtitle: request.subtitle,
            live: request.live, liveBuffer: request.live_buffer == true)
        play(item, startSeconds: request.start_seconds)
    }

    /// Lista, siguiente episodio y preferencias, sin tocar lo que se reproduce.
    private func update(_ request: PlayRequest) {
        if let list = request.items { items = list }
        currentIndex = request.current_index
        nextItem = request.next
        if self.request == nil || request.update_only != true { self.request = request }
        listButton.isHidden = items.count < 2
        listTable.reloadData()
    }

    private func play(_ item: Current, startSeconds: Double) {
        guard let url = URL(string: item.url) else {
            showMessage("No se pudo abrir este contenido.")
            return
        }
        current = item
        behindLive = 0
        pausedAt = nil
        liveReconnects = 0
        vodReconnects = 0
        errorRetries = 0
        shownForThisMedia = false
        tracksApplied = false
        nextDismissed = false
        hideNextCard()
        skipIntroButton.isHidden = true
        lastPosition = startSeconds
        titleLabel.text = item.title
        subtitleLabel.text = item.subtitle
        subtitleLabel.isHidden = (item.subtitle ?? "").isEmpty
        // Directo sin grabación: no hay nada a lo que ir hacia atrás ni adelante.
        [backButton, forwardButton].forEach { $0.isHidden = item.live && !item.liveBuffer }
        [slider, timeLabel, durationLabel].forEach { $0.isHidden = item.live }
        bottomGlass?.isHidden = item.live
        backButton.setImage(UIImage(systemName: item.liveBuffer ? "gobackward.30" : "gobackward.10", withConfiguration: bigSymbolConfig), for: .normal)
        backButton.accessibilityLabel = item.liveBuffer ? "Atrás 30 segundos" : "Atrás 10 segundos"
        updateLiveBadge()
        startPlayback(url, startSeconds: startSeconds)
        showOverlay()
        listTable.reloadData()
    }

    private func startPlayback(_ url: URL, startSeconds: Double) {
        messageLabel.isHidden = true
        spinner.startAnimating()

        let media = VLCMedia(url: url)
        // Directo con menos espera: se nota al cambiar de canal.
        media.addOption(isLive ? ":network-caching=1000" : ":network-caching=1500")
        if startSeconds > 1 {
            media.addOption(":start-time=\(Int(startSeconds))")
        }
        VibePlayerController.playGeneration += 1
        player.media = media
        player.play()
        applyFill()
    }

    /// Directo grabado: vuelve a pedir la grabación a `behindLive` segundos del directo.
    private func reopenLive() {
        guard let current = current, var components = URLComponents(string: current.url) else { return }
        var queryItems = components.queryItems ?? []
        queryItems.removeAll { $0.name == "behind_s" }
        if behindLive > 0 { queryItems.append(URLQueryItem(name: "behind_s", value: String(Int(behindLive)))) }
        components.queryItems = queryItems
        guard let url = components.url else { return }
        pausedAt = nil
        startPlayback(url, startSeconds: 0)
        updateLiveBadge()
    }

    /// Cambia a otro canal o episodio de la lista sin pasar por la interfaz web.
    private func switchTo(index: Int) {
        guard index >= 0, index < items.count, let current = current else { return }
        if !current.live { sendProgress(final: false) }
        let item = items[index]
        currentIndex = index
        play(Current(id: item.id, url: item.url, title: item.title, subtitle: item.subtitle, live: current.live, liveBuffer: current.liveBuffer), startSeconds: 0)
        sendEvent(["type": "switched", "id": item.id])
    }

    private func zap(by delta: Int) {
        guard isLive else { return }
        if let index = currentIndex, items.count > 1 {
            // Tras el último canal viene el primero (y al revés), como en TiviMate.
            switchTo(index: ((index + delta) % items.count + items.count) % items.count)
        } else if currentIndex == nil {
            sendEvent(["type": "zap", "direction": delta])
        }
    }

    private func playNext() {
        guard let next = nextItem else {
            close(ended: true)
            return
        }
        // El episodio que acaba cuenta como visto entero.
        sendProgress(final: true)
        nextItem = nil
        currentIndex = items.firstIndex { $0.id == next.id }
        play(Current(id: next.id, url: next.url, title: next.title, subtitle: next.subtitle, live: false, liveBuffer: false), startSeconds: 0)
        sendEvent(["type": "switched", "id": next.id])
    }

    /// Retraso actual respecto al directo, contando la pausa en curso.
    private var currentBehind: Double {
        behindLive + (pausedAt.map { Date().timeIntervalSince($0) } ?? 0)
    }

    private func updateLiveBadge() {
        guard isLive else {
            liveBadge.isHidden = true
            goLiveButton.isHidden = true
            return
        }
        let behind = isLiveBuffer ? currentBehind : 0
        if behind >= 5 {
            liveBadge.isHidden = true
            goLiveButton.isHidden = false
            goLiveButton.setTitle("  -\(formatTime(behind))  ·  Ir al directo  ", for: .normal)
        } else {
            liveBadge.isHidden = false
            goLiveButton.isHidden = true
        }
    }

    private var positionSeconds: Double { Double(player.time.intValue) / 1000 }

    private var durationSeconds: Double? {
        guard let length = player.media?.length.intValue, length > 0 else { return nil }
        return Double(length) / 1000
    }

    /// Pruebas del simulador (`ios/qa/e2e-qa.sh`): con `VIBE_QA_E2E`, el estado del
    /// reproductor en `qa-player.json` cada medio segundo, para comprobar posición, pausa o
    /// canal sin depender de lo que idb vea. En la app instalada esa variable no existe.
    private static let qaStateURL: URL? = {
        let env = ProcessInfo.processInfo.environment
        guard env["VIBE_QA_E2E"] != nil || env["VIBE_QA_PLAYER_URL"] != nil,
              let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first else { return nil }
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("qa-player.json")
    }()

    private func writeQaState(open: Bool) {
        guard let url = Self.qaStateURL else { return }
        var state: [String: Any] = ["open": open]
        if open, let current = current {
            state["position"] = positionSeconds
            state["playing"] = player.isPlaying
            state["live"] = current.live
            state["behind"] = isLiveBuffer ? currentBehind : 0
            state["title"] = titleLabel.text ?? ""
            state["subtitle"] = subtitleLabel.text ?? ""
            state["controls"] = overlayVisible
            state["list"] = !listPanel.isHidden
            // Tamaño de la interfaz (en horizontal, el ancho es el lado largo): la prueba
            // lo necesita para girar sus toques y no depender de que idb devuelva la pantalla.
            state["uiWidth"] = Int(controlsRoot.bounds.width)
            state["uiHeight"] = Int(controlsRoot.bounds.height)
        }
        if let data = try? JSONSerialization.data(withJSONObject: state) {
            try? data.write(to: url, options: .atomic)
        }
    }

    private func tick() {
        guard let current = current, !closed else { return }
        writeQaState(open: true)
        switch player.state {
        case .opening, .buffering:
            if !player.isPlaying { spinner.startAnimating() } else { spinner.stopAnimating() }
        case .playing:
            spinner.stopAnimating()
            if !shownForThisMedia {
                shownForThisMedia = true
                showOverlay()
            }
            if !tracksApplied { applyPreferredTracks() }
        case .error:
            // Un fallo suelto (la app volvió de segundo plano y el servidor local se reabrió,
            // un corte de red) se reintenta en el mismo punto antes de rendirse.
            if Date().timeIntervalSince(lastRetryAt) < 2 { return } // el reintento anterior aún arranca
            if errorRetries < 2, let url = URL(string: current.url) {
                errorRetries += 1
                lastRetryAt = Date()
                if current.live {
                    if current.liveBuffer { reopenLive() } else { startPlayback(url, startSeconds: 0) }
                } else {
                    startPlayback(url, startSeconds: lastPosition)
                }
                return
            }
            spinner.stopAnimating()
            showMessage("No se pudo reproducir. Puede que la cuenta esté en uso en otro dispositivo.")
            sendEvent(["type": "error", "message": "VLC no pudo abrir el stream"])
            self.current = nil
            return
        case .ended:
            if !current.live {
                // Solo es el final de verdad cerca del final: un corte a mitad (red, o el
                // proveedor) se reabre en el mismo punto en vez de salir y darla por vista.
                if let duration = durationSeconds, lastPosition < duration - 30, vodReconnects < 3, let url = URL(string: current.url) {
                    vodReconnects += 1
                    startPlayback(url, startSeconds: lastPosition)
                    return
                }
                sendProgress(final: true)
                if sleepAtEnd {
                    close(ended: true)
                } else if nextItem != nil, !nextDismissed {
                    playNext()
                } else {
                    close(ended: true)
                }
                return
            }
            // El directo se cortó (el proveedor cerró, o la grabación terminó): se vuelve a
            // pedir unas pocas veces antes de rendirse.
            if liveReconnects < 3, let url = URL(string: current.url) {
                liveReconnects += 1
                if current.liveBuffer { behindLive = 0; reopenLive() } else { startPlayback(url, startSeconds: 0) }
                return
            }
        default:
            spinner.stopAnimating()
        }

        playButton.setImage(UIImage(systemName: player.isPlaying ? "pause.fill" : "play.fill", withConfiguration: bigSymbolConfig), for: .normal)
        if player.isPlaying { liveReconnects = 0; errorRetries = 0 }
        if current.live { updateLiveBadge() }

        if !current.live {
            let position = positionSeconds
            if player.isPlaying, position > 0 { lastPosition = position; vodReconnects = 0 }
            if !scrubbing { timeLabel.text = formatTime(position) }
            if let duration = durationSeconds {
                durationLabel.text = "-" + formatTime(max(0, duration - position))
                slider.maximumValue = Float(duration)
                if !scrubbing { slider.value = Float(position) }
            }
            if player.isPlaying, Date().timeIntervalSince(lastProgressSent) >= 5 {
                sendProgress(final: false)
            }
            updateChapterButtons(position: position)
        }
        if Date().timeIntervalSince(lastNowPlaying) >= 2 { updateNowPlaying() }
    }

    private func sendProgress(final: Bool) {
        lastProgressSent = Date()
        let position = final ? (durationSeconds ?? positionSeconds) : positionSeconds
        sendEvent(["type": "progress", "position_s": position, "duration_s": durationSeconds ?? NSNull(), "id": current?.id ?? NSNull()])
    }

    private func close(ended: Bool) {
        guard !closed else { return }
        closed = true
        writeQaState(open: false)
        let position = isLive ? 0 : positionSeconds
        let duration: Any = durationSeconds ?? NSNull()
        tickTimer?.invalidate()
        hideTimer?.invalidate()
        sleepTimer?.invalidate()
        nextCountdown?.invalidate()
        player.stop()
        if let current = current, current.liveBuffer {
            VibePlayerController.releaseLiveBufferWhenStopped(current.url, generation: VibePlayerController.playGeneration)
        }
        removeControlsWindow()
        clearRemoteCommands()
        sendEvent(["type": "closed", "position_s": position, "duration_s": duration, "ended": ended, "id": current?.id ?? NSNull()])
        dismiss(animated: true) { VibePlayerController.restoreOrientation() }
    }

    /// En el iPhone el reproductor va en horizontal; al cerrarlo, la app vuelve a como se
    /// sujeta el teléfono (en vertical si no se sabe). Sin esto, toda la app se quedaba
    /// tumbada después de ver algo (visto en el simulador).
    static func restoreOrientation() {
        guard UIDevice.current.userInterfaceIdiom == .phone,
              let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }).first else { return }
        let mask: UIInterfaceOrientationMask
        switch UIDevice.current.orientation {
        case .landscapeLeft: mask = .landscapeRight
        case .landscapeRight: mask = .landscapeLeft
        default: mask = .portrait
        }
        scene.windows.forEach { $0.rootViewController?.setNeedsUpdateOfSupportedInterfaceOrientations() }
        scene.requestGeometryUpdate(.iOS(interfaceOrientations: mask)) { _ in }
    }

    /// Pide al núcleo que termine la grabación del directo, para que la conexión con el
    /// proveedor se suelte ya (regla R1: la cuenta suele admitir una sola), pero solo cuando
    /// VLC ha parado de verdad: `stop()` vuelve antes de parar y, si el núcleo cortaba la
    /// grabación con VLC aún leyendo, `--http-reconnect` la volvía a pedir y quedaba un
    /// directo abierto sin reproductor hasta cerrar la app (visto en el iPad). Si entretanto
    /// se abrió otra cosa, no se pide nada: lo nuevo manda.
    private static func releaseLiveBufferWhenStopped(_ streamURL: String, generation: Int, attempt: Int = 0) {
        guard generation == playGeneration else { return }
        let player = sharedPlayer
        let stopped = player.state == .stopped || player.state == .ended || player.state == .error
        if !stopped, attempt < 50 {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                releaseLiveBufferWhenStopped(streamURL, generation: generation, attempt: attempt + 1)
            }
            return
        }
        releaseLiveBuffer(from: streamURL)
    }

    /// Dice de qué canal (`/stream/live-buffer/channel/<id>`): si llegara tarde, el núcleo
    /// no corta el canal que se abrió después.
    private static func releaseLiveBuffer(from streamURL: String) {
        guard var components = URLComponents(string: streamURL) else { return }
        let channelId = components.path.split(separator: "/").last.flatMap { Int($0) }
        components.path = "/api/live-buffer"
        components.queryItems = channelId.map { [URLQueryItem(name: "channel_id", value: String($0))] }
        guard let url = components.url else { return }
        var request = URLRequest(url: url)
        request.httpMethod = "DELETE"
        request.timeoutInterval = 5
        URLSession.shared.dataTask(with: request).resume()
    }

    // MARK: - Capítulos: saltar intro y siguiente episodio

    private struct Chapter { let name: String; let start: Double; let end: Double }

    private func chapters() -> [Chapter] {
        let raw = player.chapterDescriptions(ofTitle: player.currentTitleIndex) as? [[String: Any]] ?? []
        return raw.compactMap { entry in
            let name = (entry[VLCChapterDescriptionName] as? String ?? "").lowercased()
            let offset = (entry[VLCChapterDescriptionTimeOffset] as? NSNumber)?.doubleValue ?? 0
            let length = (entry[VLCChapterDescriptionDuration] as? NSNumber)?.doubleValue ?? 0
            return Chapter(name: name, start: offset / 1000, end: (offset + length) / 1000)
        }
    }

    private func isIntro(_ name: String) -> Bool {
        ["intro", "opening", "cabecera", "apertura", "entradilla"].contains { name.contains($0) } || name == "op"
    }

    private func isCredits(_ name: String) -> Bool {
        ["crédito", "credito", "credits", "ending", "outro", "end"].contains { name.contains($0) } || name == "ed"
    }

    private var introEnd: Double?

    private func updateChapterButtons(position: Double) {
        let list = chapters()
        if let intro = list.first(where: { isIntro($0.name) && $0.end - $0.start <= 600 }), position >= intro.start, position < intro.end - 1 {
            introEnd = intro.end
            skipIntroButton.isHidden = locked
        } else {
            skipIntroButton.isHidden = true
        }
        guard nextItem != nil, !nextDismissed, nextCountdown == nil, let duration = durationSeconds, duration > 60 else { return }
        let creditsStart = list.first(where: { isCredits($0.name) && $0.start > duration * 0.5 })?.start ?? (duration - 30)
        if position >= creditsStart { startNextCountdown() }
    }

    @objc private func skipIntroTapped() {
        guard let end = introEnd else { return }
        player.time = VLCTime(int: Int32(end * 1000))
        skipIntroButton.isHidden = true
    }

    private func startNextCountdown() {
        guard let next = nextItem else { return }
        nextSecondsLeft = 10
        nextTitleLabel.text = "Siguiente: \(next.title)\((next.subtitle ?? "").isEmpty ? "" : " · " + (next.subtitle ?? ""))"
        nextPlayButton.setTitle("  Ver ahora (\(nextSecondsLeft))  ", for: .normal)
        nextCard.isHidden = false
        controlsRoot.bringSubviewToFront(nextCard)
        nextCountdown = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            guard let self = self else { return }
            self.nextSecondsLeft -= 1
            self.nextPlayButton.setTitle("  Ver ahora (\(max(0, self.nextSecondsLeft)))  ", for: .normal)
            if self.nextSecondsLeft <= 0 {
                self.hideNextCard()
                self.sendProgress(final: true)
                self.playNext()
            }
        }
    }

    private func hideNextCard() {
        nextCountdown?.invalidate()
        nextCountdown = nil
        nextCard.isHidden = true
    }

    @objc private func nextNowTapped() {
        hideNextCard()
        sendProgress(final: true)
        playNext()
    }

    @objc private func nextCancelTapped() {
        nextDismissed = true
        hideNextCard()
    }

    // MARK: - Pistas preferidas

    private func applyPreferredTracks() {
        guard let request = request else { tracksApplied = true; return }
        // Las pistas se conocen un instante después de empezar a reproducir.
        let audioNames: [Any] = player.audioTrackNames
        guard !audioNames.isEmpty else { return }
        tracksApplied = true
        if let wanted = request.preferred_audio?.lowercased(), !wanted.isEmpty {
            selectTrack(wanted, names: audioNames, indexes: player.audioTrackIndexes) { self.player.currentAudioTrackIndex = $0 }
        }
        if let wanted = request.preferred_subtitle?.lowercased(), !wanted.isEmpty {
            selectTrack(wanted, names: player.videoSubTitlesNames, indexes: player.videoSubTitlesIndexes) { self.player.currentVideoSubTitleIndex = $0 }
        }
    }

    private func selectTrack(_ wanted: String, names: [Any], indexes: [Any], apply: (Int32) -> Void) {
        for (name, index) in zip(names.map { "\($0)".lowercased() }, indexes.compactMap { ($0 as? NSNumber)?.int32Value }) where name.contains(wanted) {
            apply(index)
            return
        }
    }

    private func currentTrackName(names: [Any]?, indexes: [Any]?, current: Int32) -> String? {
        let pairs = zip((names ?? []).map { "\($0)" }, (indexes ?? []).compactMap { ($0 as? NSNumber)?.int32Value })
        return pairs.first { $0.1 == current && $0.1 >= 0 }?.0
    }

    private func reportTracks() {
        let audio = currentTrackName(names: player.audioTrackNames, indexes: player.audioTrackIndexes, current: player.currentAudioTrackIndex)
        let subtitle = currentTrackName(names: player.videoSubTitlesNames, indexes: player.videoSubTitlesIndexes, current: player.currentVideoSubTitleIndex)
        sendEvent(["type": "tracks", "audio": audio ?? NSNull(), "subtitle": subtitle ?? NSNull()])
    }

    // MARK: - Acciones

    @objc private func closeTapped() { close(ended: false) }

    @objc private func playTapped() {
        togglePlay()
        showOverlay()
    }

    private func togglePlay() {
        if player.isPlaying {
            player.pause()
            if isLiveBuffer { pausedAt = Date() }
        } else {
            // VLC sigue leyendo la grabación justo donde se quedó: la pausa queda como retraso.
            if let pausedAt = pausedAt {
                behindLive += Date().timeIntervalSince(pausedAt)
                self.pausedAt = nil
            }
            player.play()
        }
        updateLiveBadge()
        updateNowPlaying()
    }

    @objc private func backTapped() { seek(by: isLiveBuffer ? -30 : -10) }
    @objc private func forwardTapped() { seek(by: isLiveBuffer ? 30 : 30) }

    @objc private func goLiveTapped() {
        behindLive = 0
        reopenLive()
        showOverlay()
    }

    private func seek(by seconds: Double) {
        if isLiveBuffer {
            behindLive = max(0, currentBehind - seconds)
            reopenLive()
            showOverlay()
            return
        }
        guard !isLive else { return }
        var target = max(0, positionSeconds + seconds)
        if let duration = durationSeconds { target = min(target, duration - 1) }
        player.time = VLCTime(int: Int32(target * 1000))
        showOverlay()
    }

    @objc private func sliderBegan() { scrubbing = true; hideTimer?.invalidate() }
    @objc private func sliderMoved() { timeLabel.text = formatTime(Double(slider.value)) }

    @objc private func sliderEnded() {
        scrubbing = false
        player.time = VLCTime(int: Int32(slider.value * 1000))
        showOverlay()
    }

    @objc private func listTapped() {
        listPanel.isHidden ? showList() : hideList()
    }

    private func showList() {
        listTable.reloadData()
        listPanel.isHidden = false
        controlsRoot.bringSubviewToFront(listPanel)
        if let index = currentIndex, index < items.count {
            listTable.scrollToRow(at: IndexPath(row: index, section: 0), at: .middle, animated: false)
        }
        hideOverlay()
    }

    private func hideList() { listPanel.isHidden = true }

    @objc private func lockTapped() {
        locked = false
        lockButton.isHidden = true
        showOverlay()
        hud("Desbloqueado")
    }

    private func lock() {
        locked = true
        hideOverlay()
        hideList()
        hud("Pantalla bloqueada · toca el candado para desbloquear")
        lockButton.isHidden = false
        controlsRoot.bringSubviewToFront(lockButton)
    }

    private func setSleep(minutes: Int?, atEnd: Bool = false) {
        sleepTimer?.invalidate()
        sleepTimer = nil
        sleepAtEnd = atEnd
        if let minutes = minutes {
            sleepTimer = Timer.scheduledTimer(withTimeInterval: TimeInterval(minutes * 60), repeats: false) { [weak self] _ in
                self?.sendProgress(final: false)
                self?.close(ended: false)
            }
            hud("Se cerrará en \(minutes) min")
        } else if atEnd {
            hud("Se cerrará al terminar")
        } else {
            hud("Temporizador desactivado")
        }
    }

    @objc private func didEnterBackground() {
        // El audio sigue (decidido con el usuario); el vídeo no se puede pintar en segundo
        // plano, así que se apaga hasta volver.
        guard player.isPlaying else { return }
        savedVideoTrack = player.currentVideoTrackIndex
        player.currentVideoTrackIndex = -1
    }

    @objc private func willEnterForeground() {
        if let track = savedVideoTrack {
            player.currentVideoTrackIndex = track
            savedVideoTrack = nil
        }
    }

    @objc private func toggleFill() {
        fillScreen.toggle()
        applyFill()
        hud(fillScreen ? "Pantalla completa" : "Imagen entera")
    }

    private func applyFill() {
        // Recorta el vídeo a la proporción de la pantalla (sin bandas negras) o lo enseña
        // entero. VLC copia la cadena al recibirla.
        if fillScreen, videoView.bounds.width > 0, videoView.bounds.height > 0 {
            let geometry = strdup("\(Int(videoView.bounds.width)):\(Int(videoView.bounds.height))")
            player.videoCropGeometry = geometry
            free(geometry)
        } else {
            player.videoCropGeometry = nil
        }
        aspectButton.setImage(
            UIImage(systemName: fillScreen ? "arrow.down.right.and.arrow.up.left" : "arrow.up.left.and.arrow.down.right", withConfiguration: symbolConfig),
            for: .normal)
        aspectButton.accessibilityLabel = fillScreen ? "Ver la imagen entera" : "Llenar la pantalla"
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        if fillScreen { applyFill() }
    }

    // MARK: - Gestos

    private func buildGestures() {
        let doubleTap = UITapGestureRecognizer(target: self, action: #selector(doubleTapped(_:)))
        doubleTap.numberOfTapsRequired = 2
        doubleTap.delegate = self
        controlsRoot.addGestureRecognizer(doubleTap)
        let tap = UITapGestureRecognizer(target: self, action: #selector(toggleOverlay))
        tap.require(toFail: doubleTap)
        tap.delegate = self
        controlsRoot.addGestureRecognizer(tap)
        let pan = UIPanGestureRecognizer(target: self, action: #selector(panned(_:)))
        pan.maximumNumberOfTouches = 1
        pan.delegate = self
        controlsRoot.addGestureRecognizer(pan)
        let pinch = UIPinchGestureRecognizer(target: self, action: #selector(pinched(_:)))
        pinch.delegate = self
        controlsRoot.addGestureRecognizer(pinch)
    }

    @objc private func doubleTapped(_ gesture: UITapGestureRecognizer) {
        guard !locked else { return }
        let x = gesture.location(in: controlsRoot).x
        let width = controlsRoot.bounds.width
        if x < width / 3 {
            seek(by: -10)
            hud("−10 s")
        } else if x > width * 2 / 3 {
            seek(by: 10)
            hud("+10 s")
        } else {
            toggleFill()
        }
    }

    @objc private func pinched(_ gesture: UIPinchGestureRecognizer) {
        guard !locked, gesture.state == .ended else { return }
        if gesture.scale > 1.15, !fillScreen { toggleFill() }
        if gesture.scale < 0.85, fillScreen { toggleFill() }
    }

    @objc private func panned(_ gesture: UIPanGestureRecognizer) {
        guard !locked else { return }
        let size = controlsRoot.bounds.size
        let translation = gesture.translation(in: controlsRoot)
        switch gesture.state {
        case .began:
            let velocity = gesture.velocity(in: controlsRoot)
            let start = gesture.location(in: controlsRoot)
            if abs(velocity.x) > abs(velocity.y) {
                panMode = (isLive && !isLiveBuffer) ? .none : .seek
                panStartValue = isLiveBuffer ? currentBehind : positionSeconds
            } else if start.x < size.width * 0.25 {
                panMode = .brightness
                panStartValue = Double(UIScreen.main.brightness)
            } else if start.x > size.width * 0.75 {
                panMode = .volume
                panStartValue = Double(AVAudioSession.sharedInstance().outputVolume)
            } else {
                panMode = .zap
            }
        case .changed:
            switch panMode {
            case .seek:
                // El ancho de la pantalla equivale a 3 minutos.
                let delta = Double(translation.x / max(size.width, 1)) * 180
                if isLiveBuffer {
                    let behind = max(0, panStartValue - delta)
                    hud("-\(formatTime(behind)) del directo")
                } else {
                    var target = max(0, panStartValue + delta)
                    if let duration = durationSeconds { target = min(target, duration - 1) }
                    hud("\(delta >= 0 ? "+" : "−")\(formatTime(abs(delta)))  ·  \(formatTime(target))")
                }
            case .brightness:
                let value = min(1, max(0, panStartValue - Double(translation.y / max(size.height, 1))))
                UIScreen.main.brightness = CGFloat(value)
                hud("Brillo \(Int(value * 100)) %")
            case .volume:
                let value = min(1, max(0, panStartValue - Double(translation.y / max(size.height, 1))))
                setSystemVolume(Float(value))
                hud("Volumen \(Int(value * 100)) %")
            default:
                break
            }
        case .ended:
            switch panMode {
            case .seek:
                let delta = Double(translation.x / max(size.width, 1)) * 180
                if abs(delta) >= 2 {
                    if isLiveBuffer {
                        behindLive = max(0, panStartValue - delta)
                        reopenLive()
                    } else {
                        var target = max(0, panStartValue + delta)
                        if let duration = durationSeconds { target = min(target, duration - 1) }
                        player.time = VLCTime(int: Int32(target * 1000))
                    }
                }
            case .zap:
                if abs(translation.y) > 60 {
                    if isLive {
                        zap(by: translation.y < 0 ? 1 : -1)
                    } else if translation.y > 120 {
                        close(ended: false)
                    }
                }
            default:
                break
            }
            panMode = .none
        default:
            panMode = .none
        }
    }

    private func setSystemVolume(_ value: Float) {
        guard let slider = volumeView.subviews.compactMap({ $0 as? UISlider }).first else { return }
        slider.value = value
    }

    // MARK: - Pantalla de bloqueo y centro de control

    private func setupRemoteCommands() {
        let center = MPRemoteCommandCenter.shared()
        center.playCommand.addTarget { [weak self] _ in
            guard let self = self else { return .commandFailed }
            if !self.player.isPlaying { self.togglePlay() }
            return .success
        }
        center.pauseCommand.addTarget { [weak self] _ in
            guard let self = self else { return .commandFailed }
            if self.player.isPlaying { self.togglePlay() }
            return .success
        }
        center.togglePlayPauseCommand.addTarget { [weak self] _ in
            self?.togglePlay()
            return .success
        }
        center.skipForwardCommand.preferredIntervals = [30]
        center.skipForwardCommand.addTarget { [weak self] _ in
            self?.seek(by: 30)
            return .success
        }
        center.skipBackwardCommand.preferredIntervals = [10]
        center.skipBackwardCommand.addTarget { [weak self] _ in
            self?.seek(by: -10)
            return .success
        }
        center.changePlaybackPositionCommand.addTarget { [weak self] event in
            guard let self = self, !self.isLive, let event = event as? MPChangePlaybackPositionCommandEvent else { return .commandFailed }
            self.player.time = VLCTime(int: Int32(event.positionTime * 1000))
            return .success
        }
    }

    private func clearRemoteCommands() {
        let center = MPRemoteCommandCenter.shared()
        for command in [center.playCommand, center.pauseCommand, center.togglePlayPauseCommand, center.skipForwardCommand, center.skipBackwardCommand, center.changePlaybackPositionCommand] {
            command.removeTarget(nil)
        }
        MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
    }

    private func updateNowPlaying() {
        lastNowPlaying = Date()
        guard let current = current else { return }
        var info: [String: Any] = [
            MPMediaItemPropertyTitle: current.title,
            MPNowPlayingInfoPropertyPlaybackRate: player.isPlaying ? Double(player.rate) : 0.0,
            MPNowPlayingInfoPropertyIsLiveStream: current.live,
        ]
        if let subtitle = current.subtitle { info[MPMediaItemPropertyArtist] = subtitle }
        if !current.live {
            info[MPNowPlayingInfoPropertyElapsedPlaybackTime] = positionSeconds
            if let duration = durationSeconds { info[MPMediaItemPropertyPlaybackDuration] = duration }
        }
        MPNowPlayingInfoCenter.default().nowPlayingInfo = info
    }

    // MARK: - Menús

    private func trackMenu(
        names: @escaping () -> [Any], indexes: @escaping () -> [Any], current: @escaping () -> Int32, select: @escaping (Int32) -> Void
    ) -> UIMenu {
        UIMenu(children: [UIDeferredMenuElement.uncached { completion in
            let names = names().map { "\($0)" }
            let indexes = indexes().compactMap { ($0 as? NSNumber)?.int32Value }
            let selected = current()
            let actions = zip(names, indexes).map { name, index in
                UIAction(title: name, state: index == selected ? .on : .off) { _ in select(index) }
            }
            completion(actions.isEmpty ? [UIAction(title: "Sin pistas", attributes: .disabled) { _ in }] : actions)
        }])
    }

    private func moreMenu() -> UIMenu {
        UIMenu(children: [UIDeferredMenuElement.uncached { [weak self] completion in
            guard let self = self else { return completion([]) }
            var elements: [UIMenuElement] = []
            if !self.isLive {
                let rates: [Float] = [0.5, 0.75, 1, 1.25, 1.5, 2]
                let rateActions = rates.map { rate in
                    UIAction(title: rate == 1 ? "Normal" : "\(rate)×", state: abs(self.player.rate - rate) < 0.01 ? .on : .off) { [weak self] _ in
                        self?.player.rate = rate
                        self?.hud(rate == 1 ? "Velocidad normal" : "Velocidad \(rate)×")
                    }
                }
                elements.append(UIMenu(title: "Velocidad", image: UIImage(systemName: "speedometer"), children: rateActions))
            }
            var sleep: [UIMenuElement] = [15, 30, 45, 60, 90].map { minutes in
                UIAction(title: "\(minutes) min") { [weak self] _ in self?.setSleep(minutes: minutes) }
            }
            if !self.isLive {
                sleep.append(UIAction(title: "Al terminar") { [weak self] _ in self?.setSleep(minutes: nil, atEnd: true) })
            }
            sleep.append(UIAction(title: "Desactivar", attributes: self.sleepTimer == nil && !self.sleepAtEnd ? .disabled : []) { [weak self] _ in
                self?.setSleep(minutes: nil)
            })
            elements.append(UIMenu(title: "Temporizador de apagado", image: UIImage(systemName: "moon.zzz"), children: sleep))
            elements.append(UIAction(title: "Bloquear pantalla", image: UIImage(systemName: "lock")) { [weak self] _ in self?.lock() })
            completion(elements)
        }])
    }

    // MARK: - Interfaz

    /// Fondo de cristal (F15): el «Liquid Glass» del sistema en iOS 26; antes, el
    /// desenfoque oscuro de siempre. Solo en lo que flota sobre el vídeo.
    private func makeGlass(cornerRadius: CGFloat) -> UIVisualEffectView {
        let effect: UIVisualEffect
        if #available(iOS 26.0, *), !UIAccessibility.isReduceTransparencyEnabled {
            effect = UIGlassEffect(style: .clear)
        } else {
            effect = UIBlurEffect(style: .systemUltraThinMaterialDark)
        }
        let view = UIVisualEffectView(effect: effect)
        view.layer.cornerRadius = cornerRadius
        view.layer.cornerCurve = .continuous
        view.clipsToBounds = true
        view.translatesAutoresizingMaskIntoConstraints = false
        return view
    }

    /// Mete `content` en una pieza de cristal con los márgenes dados.
    private func glassWrap(_ content: UIView, cornerRadius: CGFloat, insets: UIEdgeInsets = .zero) -> UIVisualEffectView {
        let glass = makeGlass(cornerRadius: cornerRadius)
        content.translatesAutoresizingMaskIntoConstraints = false
        glass.contentView.addSubview(content)
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: glass.contentView.leadingAnchor, constant: insets.left),
            content.trailingAnchor.constraint(equalTo: glass.contentView.trailingAnchor, constant: -insets.right),
            content.topAnchor.constraint(equalTo: glass.contentView.topAnchor, constant: insets.top),
            content.bottomAnchor.constraint(equalTo: glass.contentView.bottomAnchor, constant: -insets.bottom),
        ])
        return glass
    }

    /// Botón redondo de cristal de `size` puntos.
    private func glassCircle(_ button: UIButton, size: CGFloat) -> UIVisualEffectView {
        let glass = glassWrap(button, cornerRadius: size / 2)
        NSLayoutConstraint.activate([
            glass.widthAnchor.constraint(equalToConstant: size),
            glass.heightAnchor.constraint(equalToConstant: size),
        ])
        return glass
    }

    private func buildOverlay() {
        controlsRoot.addSubview(volumeView)
        volumeView.alpha = 0.01

        overlay.translatesAutoresizingMaskIntoConstraints = false
        controlsRoot.addSubview(overlay)

        func style(_ button: UIButton, _ symbol: String, _ label: String, big: Bool = false) {
            button.setImage(UIImage(systemName: symbol, withConfiguration: big ? bigSymbolConfig : symbolConfig), for: .normal)
            button.tintColor = .white
            button.accessibilityLabel = label
            button.translatesAutoresizingMaskIntoConstraints = false
        }
        style(closeButton, "xmark", "Cerrar")
        style(listButton, "list.bullet", "Lista")
        style(audioButton, "speaker.wave.2", "Audio")
        style(subtitlesButton, "captions.bubble", "Subtítulos")
        style(moreButton, "ellipsis.circle", "Más opciones")
        style(playButton, "pause.fill", "Reproducir o pausar", big: true)
        style(backButton, "gobackward.10", "Atrás 10 segundos", big: true)
        style(forwardButton, "goforward.30", "Adelante 30 segundos", big: true)
        style(aspectButton, "arrow.up.left.and.arrow.down.right", "Llenar la pantalla")
        style(lockButton, "lock.fill", "Desbloquear pantalla")
        listButton.isHidden = true
        lockButton.isHidden = true
        lockButton.backgroundColor = UIColor.black.withAlphaComponent(0.5)
        lockButton.layer.cornerRadius = 24
        aspectButton.addTarget(self, action: #selector(toggleFill), for: .touchUpInside)
        listButton.addTarget(self, action: #selector(listTapped), for: .touchUpInside)
        lockButton.addTarget(self, action: #selector(lockTapped), for: .touchUpInside)
        closeButton.addTarget(self, action: #selector(closeTapped), for: .touchUpInside)
        playButton.addTarget(self, action: #selector(playTapped), for: .touchUpInside)
        backButton.addTarget(self, action: #selector(backTapped), for: .touchUpInside)
        forwardButton.addTarget(self, action: #selector(forwardTapped), for: .touchUpInside)
        goLiveButton.translatesAutoresizingMaskIntoConstraints = false
        goLiveButton.titleLabel?.font = .systemFont(ofSize: 13, weight: .bold)
        goLiveButton.tintColor = .black
        goLiveButton.backgroundColor = .white
        goLiveButton.layer.cornerRadius = 15
        goLiveButton.heightAnchor.constraint(equalToConstant: 30).isActive = true
        goLiveButton.isHidden = true
        goLiveButton.addTarget(self, action: #selector(goLiveTapped), for: .touchUpInside)

        audioButton.showsMenuAsPrimaryAction = true
        audioButton.menu = trackMenu(
            names: { [weak self] in self?.player.audioTrackNames ?? [] },
            indexes: { [weak self] in self?.player.audioTrackIndexes ?? [] },
            current: { [weak self] in self?.player.currentAudioTrackIndex ?? -1 },
            select: { [weak self] index in
                self?.player.currentAudioTrackIndex = index
                self?.reportTracks()
            })
        subtitlesButton.showsMenuAsPrimaryAction = true
        subtitlesButton.menu = trackMenu(
            names: { [weak self] in self?.player.videoSubTitlesNames ?? [] },
            indexes: { [weak self] in self?.player.videoSubTitlesIndexes ?? [] },
            current: { [weak self] in self?.player.currentVideoSubTitleIndex ?? -1 },
            select: { [weak self] index in
                self?.player.currentVideoSubTitleIndex = index
                self?.reportTracks()
            })
        moreButton.showsMenuAsPrimaryAction = true
        moreButton.menu = moreMenu()

        titleLabel.font = .systemFont(ofSize: 17, weight: .bold)
        titleLabel.textColor = .white
        subtitleLabel.font = .systemFont(ofSize: 14, weight: .regular)
        subtitleLabel.textColor = UIColor.white.withAlphaComponent(0.75)
        let titles = UIStackView(arrangedSubviews: [titleLabel, subtitleLabel])
        titles.axis = .vertical
        titles.spacing = 2
        titles.translatesAutoresizingMaskIntoConstraints = false

        liveBadge.text = "  EN DIRECTO  "
        liveBadge.font = .systemFont(ofSize: 12, weight: .bold)
        liveBadge.textColor = .white
        liveBadge.backgroundColor = .systemRed
        liveBadge.layer.cornerRadius = 9
        liveBadge.clipsToBounds = true

        let closeGlass = glassCircle(closeButton, size: 44)
        let toolButtons = [listButton, aspectButton, audioButton, subtitlesButton, moreButton]
        for button in toolButtons {
            button.widthAnchor.constraint(equalToConstant: 42).isActive = true
            button.heightAnchor.constraint(equalToConstant: 44).isActive = true
        }
        let tools = UIStackView(arrangedSubviews: toolButtons)
        tools.spacing = 2
        tools.alignment = .center
        let toolsGlass = glassWrap(tools, cornerRadius: 22, insets: UIEdgeInsets(top: 0, left: 6, bottom: 0, right: 6))
        let topBar = UIStackView(arrangedSubviews: [closeGlass, titles, liveBadge, goLiveButton, toolsGlass])
        topBar.spacing = 14
        topBar.alignment = .center
        topBar.translatesAutoresizingMaskIntoConstraints = false
        overlay.addSubview(topBar)

        let center = UIStackView(arrangedSubviews: [glassCircle(backButton, size: 64), glassCircle(playButton, size: 78), glassCircle(forwardButton, size: 64)])
        center.spacing = 36
        center.alignment = .center
        center.translatesAutoresizingMaskIntoConstraints = false
        overlay.addSubview(center)

        for label in [timeLabel, durationLabel] {
            label.font = .monospacedDigitSystemFont(ofSize: 13, weight: .medium)
            label.textColor = .white
        }
        slider.minimumTrackTintColor = .systemRed
        slider.maximumTrackTintColor = UIColor.white.withAlphaComponent(0.3)
        slider.addTarget(self, action: #selector(sliderBegan), for: .touchDown)
        slider.addTarget(self, action: #selector(sliderMoved), for: .valueChanged)
        slider.addTarget(self, action: #selector(sliderEnded), for: [.touchUpInside, .touchUpOutside, .touchCancel])
        let bottomBar = UIStackView(arrangedSubviews: [timeLabel, slider, durationLabel])
        bottomBar.spacing = 12
        bottomBar.alignment = .center
        let bottomGlass = glassWrap(bottomBar, cornerRadius: 24, insets: UIEdgeInsets(top: 6, left: 18, bottom: 6, right: 18))
        bottomGlass.heightAnchor.constraint(equalToConstant: 48).isActive = true
        overlay.addSubview(bottomGlass)
        self.bottomGlass = bottomGlass

        spinner.color = .white
        spinner.hidesWhenStopped = true
        spinner.translatesAutoresizingMaskIntoConstraints = false
        controlsRoot.addSubview(spinner)

        messageLabel.textColor = .white
        messageLabel.numberOfLines = 0
        messageLabel.textAlignment = .center
        messageLabel.isHidden = true
        messageLabel.translatesAutoresizingMaskIntoConstraints = false
        controlsRoot.addSubview(messageLabel)

        hudLabel.font = .systemFont(ofSize: 15, weight: .semibold)
        hudLabel.textColor = .white
        hudLabel.backgroundColor = UIColor.black.withAlphaComponent(0.55)
        hudLabel.layer.cornerRadius = 18
        hudLabel.layer.cornerCurve = .continuous
        hudLabel.clipsToBounds = true
        hudLabel.alpha = 0
        hudLabel.translatesAutoresizingMaskIntoConstraints = false
        controlsRoot.addSubview(hudLabel)

        skipIntroButton.setTitle("  Saltar intro  ", for: .normal)
        skipIntroButton.titleLabel?.font = .systemFont(ofSize: 15, weight: .bold)
        skipIntroButton.tintColor = .black
        skipIntroButton.backgroundColor = .white
        skipIntroButton.layer.cornerRadius = 22
        skipIntroButton.isHidden = true
        skipIntroButton.translatesAutoresizingMaskIntoConstraints = false
        skipIntroButton.addTarget(self, action: #selector(skipIntroTapped), for: .touchUpInside)
        controlsRoot.addSubview(skipIntroButton)

        nextCard.backgroundColor = .clear
        nextCard.layer.cornerRadius = 22
        nextCard.layer.cornerCurve = .continuous
        nextCard.clipsToBounds = true
        let nextGlass = makeGlass(cornerRadius: 22)
        nextCard.addSubview(nextGlass)
        NSLayoutConstraint.activate([
            nextGlass.leadingAnchor.constraint(equalTo: nextCard.leadingAnchor),
            nextGlass.trailingAnchor.constraint(equalTo: nextCard.trailingAnchor),
            nextGlass.topAnchor.constraint(equalTo: nextCard.topAnchor),
            nextGlass.bottomAnchor.constraint(equalTo: nextCard.bottomAnchor),
        ])
        nextCard.isHidden = true
        nextCard.translatesAutoresizingMaskIntoConstraints = false
        nextTitleLabel.font = .systemFont(ofSize: 14, weight: .semibold)
        nextTitleLabel.textColor = .white
        nextTitleLabel.numberOfLines = 2
        nextPlayButton.titleLabel?.font = .systemFont(ofSize: 15, weight: .bold)
        nextPlayButton.tintColor = .black
        nextPlayButton.backgroundColor = .white
        nextPlayButton.layer.cornerRadius = 20
        nextPlayButton.addTarget(self, action: #selector(nextNowTapped), for: .touchUpInside)
        nextCancelButton.setTitle("Cancelar", for: .normal)
        nextCancelButton.tintColor = .white
        nextCancelButton.addTarget(self, action: #selector(nextCancelTapped), for: .touchUpInside)
        let nextButtons = UIStackView(arrangedSubviews: [nextPlayButton, nextCancelButton])
        nextButtons.spacing = 12
        let nextStack = UIStackView(arrangedSubviews: [nextTitleLabel, nextButtons])
        nextStack.axis = .vertical
        nextStack.spacing = 10
        nextStack.translatesAutoresizingMaskIntoConstraints = false
        nextCard.addSubview(nextStack)
        controlsRoot.addSubview(nextCard)

        listPanel.backgroundColor = UIColor.black.withAlphaComponent(0.25)
        listPanel.layer.cornerRadius = 28
        listPanel.layer.cornerCurve = .continuous
        listPanel.layer.maskedCorners = [.layerMinXMinYCorner, .layerMinXMaxYCorner]
        listPanel.clipsToBounds = true
        let listGlass = makeGlass(cornerRadius: 0)
        listPanel.addSubview(listGlass)
        NSLayoutConstraint.activate([
            listGlass.leadingAnchor.constraint(equalTo: listPanel.leadingAnchor),
            listGlass.trailingAnchor.constraint(equalTo: listPanel.trailingAnchor),
            listGlass.topAnchor.constraint(equalTo: listPanel.topAnchor),
            listGlass.bottomAnchor.constraint(equalTo: listPanel.bottomAnchor),
        ])
        listPanel.isHidden = true
        listPanel.translatesAutoresizingMaskIntoConstraints = false
        listTable.backgroundColor = .clear
        listTable.separatorColor = UIColor(white: 1, alpha: 0.08)
        listTable.dataSource = self
        listTable.delegate = self
        listTable.translatesAutoresizingMaskIntoConstraints = false
        listPanel.addSubview(listTable)
        controlsRoot.addSubview(listPanel)
        controlsRoot.addSubview(lockButton)

        let root = controlsRoot
        let guide = root.safeAreaLayoutGuide
        NSLayoutConstraint.activate([
            overlay.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            overlay.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            overlay.topAnchor.constraint(equalTo: root.topAnchor),
            overlay.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            topBar.leadingAnchor.constraint(equalTo: guide.leadingAnchor, constant: 16),
            topBar.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -16),
            topBar.topAnchor.constraint(equalTo: guide.topAnchor, constant: 12),
            center.centerXAnchor.constraint(equalTo: root.centerXAnchor),
            center.centerYAnchor.constraint(equalTo: root.centerYAnchor),
            bottomGlass.leadingAnchor.constraint(equalTo: guide.leadingAnchor, constant: 16),
            bottomGlass.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -16),
            bottomGlass.bottomAnchor.constraint(equalTo: guide.bottomAnchor, constant: -10),
            spinner.centerXAnchor.constraint(equalTo: root.centerXAnchor),
            spinner.centerYAnchor.constraint(equalTo: root.centerYAnchor),
            messageLabel.centerXAnchor.constraint(equalTo: root.centerXAnchor),
            messageLabel.centerYAnchor.constraint(equalTo: root.centerYAnchor, constant: 70),
            messageLabel.leadingAnchor.constraint(greaterThanOrEqualTo: guide.leadingAnchor, constant: 32),
            messageLabel.trailingAnchor.constraint(lessThanOrEqualTo: guide.trailingAnchor, constant: -32),
            hudLabel.centerXAnchor.constraint(equalTo: root.centerXAnchor),
            hudLabel.topAnchor.constraint(equalTo: guide.topAnchor, constant: 64),
            skipIntroButton.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -24),
            skipIntroButton.bottomAnchor.constraint(equalTo: guide.bottomAnchor, constant: -64),
            skipIntroButton.heightAnchor.constraint(equalToConstant: 44),
            nextCard.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -24),
            nextCard.bottomAnchor.constraint(equalTo: guide.bottomAnchor, constant: -64),
            nextCard.widthAnchor.constraint(equalToConstant: 300),
            nextStack.leadingAnchor.constraint(equalTo: nextCard.leadingAnchor, constant: 14),
            nextStack.trailingAnchor.constraint(equalTo: nextCard.trailingAnchor, constant: -14),
            nextStack.topAnchor.constraint(equalTo: nextCard.topAnchor, constant: 12),
            nextStack.bottomAnchor.constraint(equalTo: nextCard.bottomAnchor, constant: -12),
            nextPlayButton.heightAnchor.constraint(equalToConstant: 40),
            listPanel.topAnchor.constraint(equalTo: root.topAnchor),
            listPanel.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            listPanel.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            listPanel.widthAnchor.constraint(equalTo: root.widthAnchor, multiplier: 0.42),
            listTable.topAnchor.constraint(equalTo: guide.topAnchor, constant: 8),
            listTable.bottomAnchor.constraint(equalTo: guide.bottomAnchor),
            listTable.leadingAnchor.constraint(equalTo: listPanel.leadingAnchor),
            listTable.trailingAnchor.constraint(equalTo: guide.trailingAnchor),
            lockButton.leadingAnchor.constraint(equalTo: guide.leadingAnchor, constant: 24),
            lockButton.bottomAnchor.constraint(equalTo: guide.bottomAnchor, constant: -24),
            lockButton.widthAnchor.constraint(equalToConstant: 48),
            lockButton.heightAnchor.constraint(equalToConstant: 48),
        ])
        titles.setContentHuggingPriority(.defaultLow, for: .horizontal)
        titles.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        // Los botones de la barra de arriba, a su tamaño: el título se queda con el resto
        // (antes "Cerrar" se estiraba a media pantalla).
        for button in [closeGlass, liveBadge, goLiveButton, toolsGlass] as [UIView] {
            button.setContentHuggingPriority(.required, for: .horizontal)
            button.setContentCompressionResistancePriority(.required, for: .horizontal)
        }
    }

    private func showMessage(_ text: String) {
        messageLabel.text = text
        messageLabel.isHidden = false
        showOverlay(autoHide: false)
    }

    private func hud(_ text: String) {
        hudLabel.text = text
        controlsRoot.bringSubviewToFront(hudLabel)
        hudTimer?.invalidate()
        UIView.animate(withDuration: 0.12) { self.hudLabel.alpha = 1 }
        hudTimer = Timer.scheduledTimer(withTimeInterval: 1.2, repeats: false) { [weak self] _ in
            UIView.animate(withDuration: 0.25) { self?.hudLabel.alpha = 0 }
        }
    }

    @objc private func toggleOverlay() {
        if locked {
            lockButton.isHidden = false
            controlsRoot.bringSubviewToFront(lockButton)
            return
        }
        if !listPanel.isHidden {
            hideList()
            return
        }
        if overlayVisible { hideOverlay() } else { showOverlay() }
    }

    private var overlayVisible: Bool { !overlay.isHidden && overlay.alpha > 0.5 }

    private func showOverlay(autoHide: Bool = true) {
        guard !locked else { return }
        overlay.isHidden = false
        controlsRoot.bringSubviewToFront(overlay)
        for view in [skipIntroButton, nextCard, listPanel, hudLabel, lockButton] as [UIView] { controlsRoot.bringSubviewToFront(view) }
        UIView.animate(withDuration: 0.2) { self.overlay.alpha = 1 }
        hideTimer?.invalidate()
        guard autoHide else { return }
        scheduleHide(after: Self.controlsHideDelay)
    }

    /// Los controles se esconden a los 4 s. En la prueba de punta a punta (`VIBE_QA_E2E`),
    /// a los 15: cada consulta a idb tarda 2 o 3 s y, si no, el toque llegaba cuando ya se
    /// habían escondido. La prueba corta del reproductor sigue comprobando los 4 s.
    private static let controlsHideDelay: TimeInterval =
        ProcessInfo.processInfo.environment["VIBE_QA_E2E"] != nil ? 15 : 4

    /// Esconde los controles cuando el vídeo esté en marcha; mientras carga o está en pausa
    /// se quedan, y se vuelve a mirar al rato.
    private func scheduleHide(after seconds: TimeInterval) {
        hideTimer?.invalidate()
        hideTimer = Timer.scheduledTimer(withTimeInterval: seconds, repeats: false) { [weak self] _ in
            guard let self = self, !self.closed else { return }
            if self.player.isPlaying, !self.scrubbing, self.pausedAt == nil {
                self.hideOverlay()
            } else if self.player.state != .paused {
                self.scheduleHide(after: 2)
            }
        }
    }

    private func hideOverlay() {
        // Oculto de verdad al acabar (no solo transparente): fuera también para la
        // accesibilidad y los toques.
        UIView.animate(withDuration: 0.3, animations: { self.overlay.alpha = 0 }) { _ in
            if self.overlay.alpha < 0.01 { self.overlay.isHidden = true }
        }
    }

    private func formatTime(_ seconds: Double) -> String {
        let total = Int(max(0, seconds))
        let h = total / 3600, m = (total % 3600) / 60, s = total % 60
        return h > 0 ? String(format: "%d:%02d:%02d", h, m, s) : String(format: "%d:%02d", m, s)
    }
}

// MARK: - Gestos: no robar los toques a botones, deslizadores ni a la lista

extension VibePlayerController: UIGestureRecognizerDelegate {
    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        guard let view = touch.view else { return true }
        if view is UIControl { return false }
        for panel in [listPanel, nextCard] where !panel.isHidden && view.isDescendant(of: panel) {
            return false
        }
        return true
    }
}

// MARK: - Lista de canales o episodios

extension VibePlayerController: UITableViewDataSource, UITableViewDelegate {
    func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { items.count }

    func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = tableView.dequeueReusableCell(withIdentifier: "item") ?? UITableViewCell(style: .subtitle, reuseIdentifier: "item")
        let item = items[indexPath.row]
        let number = item.number.map { "\($0)  " } ?? ""
        cell.textLabel?.text = number + item.title
        cell.textLabel?.textColor = .white
        cell.textLabel?.font = .systemFont(ofSize: 16, weight: indexPath.row == currentIndex ? .bold : .regular)
        cell.detailTextLabel?.text = item.subtitle
        cell.detailTextLabel?.textColor = UIColor.white.withAlphaComponent(0.6)
        cell.backgroundColor = indexPath.row == currentIndex ? UIColor.systemRed.withAlphaComponent(0.35) : .clear
        let selected = UIView()
        selected.backgroundColor = UIColor.white.withAlphaComponent(0.12)
        cell.selectedBackgroundView = selected
        return cell
    }

    func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        hideList()
        if indexPath.row != currentIndex { switchTo(index: indexPath.row) }
    }
}

/// Etiqueta con margen interior, para los avisos flotantes.
private final class PaddedLabel: UILabel {
    override func drawText(in rect: CGRect) {
        super.drawText(in: rect.inset(by: UIEdgeInsets(top: 8, left: 14, bottom: 8, right: 14)))
    }

    override var intrinsicContentSize: CGSize {
        let size = super.intrinsicContentSize
        return CGSize(width: size.width + 28, height: size.height + 16)
    }
}
