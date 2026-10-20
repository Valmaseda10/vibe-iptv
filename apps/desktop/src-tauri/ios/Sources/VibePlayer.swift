// Reproductor nativo de iPhone y iPad (F15): VLC a pantalla completa por encima de la
// interfaz web. Ver `src/native_player.rs` para el puente con el núcleo.
//
// Lo que entra: `vibe_player_present(json)`, llamado desde Rust.
// Lo que sale: `vibe_player_event(json)`, implementado en Rust, que lo reenvía a la
// interfaz. Los eventos son los de `NativePlayerEvent` en `native_player.rs`.

import AVFoundation
import MobileVLCKit
import UIKit

@_silgen_name("vibe_player_event")
private func vibePlayerEventToRust(_ json: UnsafePointer<CChar>)

private func sendEvent(_ payload: [String: Any]) {
    guard let data = try? JSONSerialization.data(withJSONObject: payload),
          let text = String(data: data, encoding: .utf8) else { return }
    text.withCString { vibePlayerEventToRust($0) }
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

final class VibePlayerController: UIViewController {
    private static weak var current: VibePlayerController?

    /// Abre el reproductor, o cambia lo que reproduce si ya está abierto (zapping).
    fileprivate static func show(_ request: PlayRequest) {
        if let controller = current, controller.presentingViewController != nil {
            if let message = request.message {
                controller.spinner.stopAnimating()
                controller.showMessage(message)
            } else {
                controller.load(request)
            }
            return
        }
        if request.message != nil { return }
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
    private let player = VLCMediaPlayer(options: ["--http-reconnect"])
    private var request: PlayRequest?
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
    private let overlay = UIView()
    private let titleLabel = UILabel()
    private let subtitleLabel = UILabel()
    private let closeButton = UIButton(type: .system)
    private let audioButton = UIButton(type: .system)
    private let subtitlesButton = UIButton(type: .system)
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

    private var tickTimer: Timer?
    private var hideTimer: Timer?
    private var lastProgressSent = Date.distantPast
    private var scrubbing = false
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
    /// Rellenar la pantalla recortando los bordes, en vez de ver la imagen entera.
    private var fillScreen = false
    private let symbolConfig = UIImage.SymbolConfiguration(pointSize: 22, weight: .semibold)
    private let bigSymbolConfig = UIImage.SymbolConfiguration(pointSize: 40, weight: .semibold)

    private var isLiveBuffer: Bool { request?.live == true && request?.live_buffer == true }

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

        buildOverlay()
        NSLayoutConstraint.activate([
            videoView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            videoView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            videoView.topAnchor.constraint(equalTo: view.topAnchor),
            videoView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])

        // Doble toque: llenar la pantalla o ver la imagen entera. El toque simple espera a
        // que no sea doble.
        let doubleTap = UITapGestureRecognizer(target: self, action: #selector(toggleFill))
        doubleTap.numberOfTapsRequired = 2
        controlsRoot.addGestureRecognizer(doubleTap)
        let tap = UITapGestureRecognizer(target: self, action: #selector(toggleOverlay))
        tap.require(toFail: doubleTap)
        controlsRoot.addGestureRecognizer(tap)
        for direction in [UISwipeGestureRecognizer.Direction.up, .down] {
            let swipe = UISwipeGestureRecognizer(target: self, action: #selector(swiped(_:)))
            swipe.direction = direction
            controlsRoot.addGestureRecognizer(swipe)
        }

        NotificationCenter.default.addObserver(
            self, selector: #selector(willResignActive), name: UIApplication.willResignActiveNotification, object: nil)
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
        DispatchQueue.main.asyncAfter(deadline: .now() + 6) { [weak self] in self?.qaDumpHierarchy("6 s tras abrir") }
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
        guard let url = URL(string: request.url) else {
            showMessage("No se pudo abrir este contenido.")
            return
        }
        self.request = request
        behindLive = 0
        pausedAt = nil
        liveReconnects = 0
        vodReconnects = 0
        errorRetries = 0
        shownForThisMedia = false
        lastPosition = request.start_seconds
        titleLabel.text = request.title
        subtitleLabel.text = request.subtitle
        subtitleLabel.isHidden = (request.subtitle ?? "").isEmpty
        let liveBuffer = request.live && request.live_buffer == true
        // Directo sin grabación: no hay nada a lo que ir hacia atrás ni adelante.
        [backButton, forwardButton].forEach { $0.isHidden = request.live && !liveBuffer }
        [slider, timeLabel, durationLabel].forEach { $0.isHidden = request.live }
        backButton.setImage(UIImage(systemName: liveBuffer ? "gobackward.30" : "gobackward.10", withConfiguration: bigSymbolConfig), for: .normal)
        backButton.accessibilityLabel = liveBuffer ? "Atrás 30 segundos" : "Atrás 10 segundos"
        updateLiveBadge()
        startPlayback(url, startSeconds: request.start_seconds)
        showOverlay()
    }

    private func startPlayback(_ url: URL, startSeconds: Double) {
        if player.media != nil { player.stop() }
        messageLabel.isHidden = true
        spinner.startAnimating()

        let media = VLCMedia(url: url)
        media.addOption(":network-caching=1500")
        if startSeconds > 1 {
            media.addOption(":start-time=\(Int(startSeconds))")
        }
        player.media = media
        player.play()
        applyFill()
    }

    /// Directo grabado: vuelve a pedir la grabación a `behindLive` segundos del directo.
    private func reopenLive() {
        guard let request = request, var components = URLComponents(string: request.url) else { return }
        var items = components.queryItems ?? []
        items.removeAll { $0.name == "behind_s" }
        if behindLive > 0 { items.append(URLQueryItem(name: "behind_s", value: String(Int(behindLive)))) }
        components.queryItems = items
        guard let url = components.url else { return }
        pausedAt = nil
        startPlayback(url, startSeconds: 0)
        updateLiveBadge()
    }

    /// Retraso actual respecto al directo, contando la pausa en curso.
    private var currentBehind: Double {
        behindLive + (pausedAt.map { Date().timeIntervalSince($0) } ?? 0)
    }

    private func updateLiveBadge() {
        guard let request = request, request.live else {
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

    private func tick() {
        guard let request = request, !closed else { return }
        switch player.state {
        case .opening, .buffering:
            if !player.isPlaying { spinner.startAnimating() } else { spinner.stopAnimating() }
        case .playing:
            spinner.stopAnimating()
            if !shownForThisMedia {
                shownForThisMedia = true
                showOverlay()
            }
        case .error:
            // Un fallo suelto (la app volvió de segundo plano y el servidor local se reabrió,
            // un corte de red) se reintenta en el mismo punto antes de rendirse.
            if Date().timeIntervalSince(lastRetryAt) < 2 { return } // el reintento anterior aún arranca
            if errorRetries < 2 {
                errorRetries += 1
                lastRetryAt = Date()
                if request.live {
                    if isLiveBuffer { reopenLive() } else if let url = URL(string: request.url) { startPlayback(url, startSeconds: 0) }
                } else if let url = URL(string: request.url) {
                    startPlayback(url, startSeconds: lastPosition)
                }
                return
            }
            spinner.stopAnimating()
            showMessage("No se pudo reproducir. Puede que la cuenta esté en uso en otro dispositivo.")
            sendEvent(["type": "error", "message": "VLC no pudo abrir el stream"])
            self.request = nil
            return
        case .ended:
            if !request.live {
                // Solo es el final de verdad cerca del final: un corte a mitad (red, o el
                // proveedor) se reabre en el mismo punto en vez de salir y darla por vista.
                if let duration = durationSeconds, lastPosition < duration - 30, vodReconnects < 3, let url = URL(string: request.url) {
                    vodReconnects += 1
                    startPlayback(url, startSeconds: lastPosition)
                    return
                }
                close(ended: true)
                return
            }
            // El directo se cortó (el proveedor cerró, o la grabación terminó): se vuelve a
            // pedir unas pocas veces antes de rendirse.
            if liveReconnects < 3 {
                liveReconnects += 1
                if isLiveBuffer { behindLive = 0; reopenLive() } else if let url = URL(string: request.url) { startPlayback(url, startSeconds: 0) }
                return
            }
        default:
            spinner.stopAnimating()
        }

        playButton.setImage(UIImage(systemName: player.isPlaying ? "pause.fill" : "play.fill", withConfiguration: bigSymbolConfig), for: .normal)
        if player.isPlaying { liveReconnects = 0; errorRetries = 0 }
        if request.live { updateLiveBadge() }

        if !request.live {
            let position = positionSeconds
            if player.isPlaying, position > 0 { lastPosition = position; vodReconnects = 0 }
            timeLabel.text = formatTime(position)
            if let duration = durationSeconds {
                durationLabel.text = formatTime(duration)
                slider.maximumValue = Float(duration)
                if !scrubbing { slider.value = Float(position) }
            }
            if player.isPlaying, Date().timeIntervalSince(lastProgressSent) >= 5 {
                lastProgressSent = Date()
                sendEvent(["type": "progress", "position_s": position, "duration_s": durationSeconds ?? NSNull()])
            }
        }
    }

    private func close(ended: Bool) {
        guard !closed else { return }
        closed = true
        let position = request?.live == true ? 0 : positionSeconds
        let duration: Any = durationSeconds ?? NSNull()
        tickTimer?.invalidate()
        hideTimer?.invalidate()
        player.stop()
        removeControlsWindow()
        sendEvent(["type": "closed", "position_s": position, "duration_s": duration, "ended": ended])
        dismiss(animated: true)
    }

    // MARK: - Acciones

    @objc private func closeTapped() { close(ended: false) }

    @objc private func playTapped() {
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
        showOverlay()
    }

    @objc private func backTapped() { seek(by: isLiveBuffer ? -30 : -10) }
    @objc private func forwardTapped() { seek(by: 30) }

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

    @objc private func swiped(_ gesture: UISwipeGestureRecognizer) {
        guard request?.live == true else {
            // En películas y series, deslizar hacia abajo cierra, como en cualquier
            // reproductor de iPhone: otra salida aunque no se vean los controles.
            if gesture.direction == .down { close(ended: false) }
            return
        }
        // Deslizar hacia arriba pasa al canal siguiente, como en un mando.
        sendEvent(["type": "zap", "direction": gesture.direction == .up ? 1 : -1])
    }

    @objc private func willResignActive() {
        if player.isPlaying {
            player.pause()
            if isLiveBuffer { pausedAt = Date() }
        }
    }

    @objc private func toggleFill() {
        fillScreen.toggle()
        applyFill()
        showOverlay()
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

    // MARK: - Pistas

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

    // MARK: - Interfaz

    private func buildOverlay() {
        overlay.translatesAutoresizingMaskIntoConstraints = false
        overlay.backgroundColor = UIColor.black.withAlphaComponent(0.35)
        controlsRoot.addSubview(overlay)

        let config = symbolConfig
        let bigConfig = bigSymbolConfig
        func style(_ button: UIButton, _ symbol: String, _ label: String, big: Bool = false) {
            button.setImage(UIImage(systemName: symbol, withConfiguration: big ? bigConfig : config), for: .normal)
            button.tintColor = .white
            button.accessibilityLabel = label
            button.translatesAutoresizingMaskIntoConstraints = false
        }
        style(closeButton, "xmark", "Cerrar")
        style(audioButton, "speaker.wave.2", "Audio")
        style(subtitlesButton, "captions.bubble", "Subtítulos")
        style(playButton, "pause.fill", "Reproducir o pausar", big: true)
        style(backButton, "gobackward.10", "Atrás 10 segundos", big: true)
        style(forwardButton, "goforward.30", "Adelante 30 segundos", big: true)
        style(aspectButton, "arrow.up.left.and.arrow.down.right", "Llenar la pantalla")
        aspectButton.addTarget(self, action: #selector(toggleFill), for: .touchUpInside)
        goLiveButton.translatesAutoresizingMaskIntoConstraints = false
        goLiveButton.titleLabel?.font = .systemFont(ofSize: 13, weight: .bold)
        goLiveButton.tintColor = .white
        goLiveButton.backgroundColor = UIColor.white.withAlphaComponent(0.2)
        goLiveButton.layer.cornerRadius = 4
        goLiveButton.isHidden = true
        goLiveButton.addTarget(self, action: #selector(goLiveTapped), for: .touchUpInside)
        closeButton.addTarget(self, action: #selector(closeTapped), for: .touchUpInside)
        playButton.addTarget(self, action: #selector(playTapped), for: .touchUpInside)
        backButton.addTarget(self, action: #selector(backTapped), for: .touchUpInside)
        forwardButton.addTarget(self, action: #selector(forwardTapped), for: .touchUpInside)

        audioButton.showsMenuAsPrimaryAction = true
        audioButton.menu = trackMenu(
            names: { [weak self] in self?.player.audioTrackNames ?? [] },
            indexes: { [weak self] in self?.player.audioTrackIndexes ?? [] },
            current: { [weak self] in self?.player.currentAudioTrackIndex ?? -1 },
            select: { [weak self] index in self?.player.currentAudioTrackIndex = index })
        subtitlesButton.showsMenuAsPrimaryAction = true
        subtitlesButton.menu = trackMenu(
            names: { [weak self] in self?.player.videoSubTitlesNames ?? [] },
            indexes: { [weak self] in self?.player.videoSubTitlesIndexes ?? [] },
            current: { [weak self] in self?.player.currentVideoSubTitleIndex ?? -1 },
            select: { [weak self] index in self?.player.currentVideoSubTitleIndex = index })

        titleLabel.font = .preferredFont(forTextStyle: .headline)
        titleLabel.textColor = .white
        subtitleLabel.font = .preferredFont(forTextStyle: .subheadline)
        subtitleLabel.textColor = UIColor.white.withAlphaComponent(0.75)
        let titles = UIStackView(arrangedSubviews: [titleLabel, subtitleLabel])
        titles.axis = .vertical
        titles.translatesAutoresizingMaskIntoConstraints = false

        liveBadge.text = "  EN DIRECTO  "
        liveBadge.font = .systemFont(ofSize: 12, weight: .bold)
        liveBadge.textColor = .white
        liveBadge.backgroundColor = .systemRed
        liveBadge.layer.cornerRadius = 4
        liveBadge.clipsToBounds = true

        let topBar = UIStackView(arrangedSubviews: [closeButton, titles, liveBadge, goLiveButton, aspectButton, audioButton, subtitlesButton])
        topBar.spacing = 16
        topBar.alignment = .center
        topBar.translatesAutoresizingMaskIntoConstraints = false
        overlay.addSubview(topBar)

        let center = UIStackView(arrangedSubviews: [backButton, playButton, forwardButton])
        center.spacing = 48
        center.alignment = .center
        center.translatesAutoresizingMaskIntoConstraints = false
        overlay.addSubview(center)

        for label in [timeLabel, durationLabel] {
            label.font = .monospacedDigitSystemFont(ofSize: 13, weight: .medium)
            label.textColor = .white
        }
        slider.minimumTrackTintColor = .systemRed
        slider.addTarget(self, action: #selector(sliderBegan), for: .touchDown)
        slider.addTarget(self, action: #selector(sliderMoved), for: .valueChanged)
        slider.addTarget(self, action: #selector(sliderEnded), for: [.touchUpInside, .touchUpOutside, .touchCancel])
        let bottomBar = UIStackView(arrangedSubviews: [timeLabel, slider, durationLabel])
        bottomBar.spacing = 12
        bottomBar.alignment = .center
        bottomBar.translatesAutoresizingMaskIntoConstraints = false
        overlay.addSubview(bottomBar)

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
            bottomBar.leadingAnchor.constraint(equalTo: guide.leadingAnchor, constant: 16),
            bottomBar.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -16),
            bottomBar.bottomAnchor.constraint(equalTo: guide.bottomAnchor, constant: -12),
            spinner.centerXAnchor.constraint(equalTo: root.centerXAnchor),
            spinner.centerYAnchor.constraint(equalTo: root.centerYAnchor),
            messageLabel.centerXAnchor.constraint(equalTo: root.centerXAnchor),
            messageLabel.centerYAnchor.constraint(equalTo: root.centerYAnchor, constant: 60),
            messageLabel.leadingAnchor.constraint(greaterThanOrEqualTo: guide.leadingAnchor, constant: 32),
            messageLabel.trailingAnchor.constraint(lessThanOrEqualTo: guide.trailingAnchor, constant: -32),
        ])
        titles.setContentHuggingPriority(.defaultLow, for: .horizontal)
        titles.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        // Los botones de la barra de arriba, a su tamaño: el título se queda con el resto
        // (antes "Cerrar" se estiraba a media pantalla).
        for button in [closeButton, liveBadge, goLiveButton, aspectButton, audioButton, subtitlesButton] as [UIView] {
            button.setContentHuggingPriority(.required, for: .horizontal)
            button.setContentCompressionResistancePriority(.required, for: .horizontal)
        }
    }

    private func showMessage(_ text: String) {
        messageLabel.text = text
        messageLabel.isHidden = false
        showOverlay(autoHide: false)
    }

    /// Solo en la prueba automática del simulador (ver `ios/qa/player-qa.sh`): vuelca qué
    /// vistas hay y en qué orden, para saber qué tapa a qué.
    private static let qaMode = ProcessInfo.processInfo.environment["VIBE_QA_PLAYER_URL"] != nil

    private func qaDumpHierarchy(_ label: String) {
        guard Self.qaMode else { return }
        var lines = ["VIBE-QA jerarquía (\(label))"]
        func walk(_ v: UIView, _ depth: Int) {
            let pad = String(repeating: "  ", count: depth)
            lines.append("\(pad)\(type(of: v)) frame=\(v.frame.integral) alpha=\(v.alpha) hidden=\(v.isHidden) touch=\(v.isUserInteractionEnabled) gestos=\(v.gestureRecognizers?.count ?? 0)")
            for sub in v.subviews { walk(sub, depth + 1) }
        }
        for scene in UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }) {
            for window in scene.windows {
                lines.append("ventana \(type(of: window)) nivel=\(window.windowLevel.rawValue) clave=\(window.isKeyWindow)")
                walk(window, 1)
            }
        }
        for line in lines { NSLog("%@", line) }
    }

    @objc private func toggleOverlay() {
        if Self.qaMode { NSLog("VIBE-QA toque recibido, controles visibles=%d", overlayVisible ? 1 : 0) }
        if overlayVisible { hideOverlay() } else { showOverlay() }
    }

    private var overlayVisible: Bool { !overlay.isHidden && overlay.alpha > 0.5 }

    private func showOverlay(autoHide: Bool = true) {
        overlay.isHidden = false
        controlsRoot.bringSubviewToFront(overlay)
        UIView.animate(withDuration: 0.2) { self.overlay.alpha = 1 }
        hideTimer?.invalidate()
        guard autoHide else { return }
        scheduleHide(after: 4)
    }

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
