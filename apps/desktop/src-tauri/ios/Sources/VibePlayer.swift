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

final class VibePlayerController: UIViewController {
    private static weak var current: VibePlayerController?

    /// Abre el reproductor, o cambia lo que reproduce si ya está abierto (zapping).
    fileprivate static func show(_ request: PlayRequest) {
        if let controller = current, controller.presentingViewController != nil {
            controller.load(request)
            return
        }
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
    private let spinner = UIActivityIndicatorView(style: .large)
    private let messageLabel = UILabel()

    private var tickTimer: Timer?
    private var hideTimer: Timer?
    private var lastProgressSent = Date.distantPast
    private var scrubbing = false

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
        view.addSubview(videoView)
        player.drawable = videoView

        buildOverlay()
        NSLayoutConstraint.activate([
            videoView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            videoView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            videoView.topAnchor.constraint(equalTo: view.topAnchor),
            videoView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])

        let tap = UITapGestureRecognizer(target: self, action: #selector(toggleOverlay))
        view.addGestureRecognizer(tap)
        for direction in [UISwipeGestureRecognizer.Direction.up, .down] {
            let swipe = UISwipeGestureRecognizer(target: self, action: #selector(swiped(_:)))
            swipe.direction = direction
            view.addGestureRecognizer(swipe)
        }

        NotificationCenter.default.addObserver(
            self, selector: #selector(willResignActive), name: UIApplication.willResignActiveNotification, object: nil)
        tickTimer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in self?.tick() }
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        UIApplication.shared.isIdleTimerDisabled = true
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        UIApplication.shared.isIdleTimerDisabled = false
    }

    // MARK: - Reproducción

    private func load(_ request: PlayRequest) {
        guard let url = URL(string: request.url) else {
            showMessage("No se pudo abrir este contenido.")
            return
        }
        self.request = request
        if player.media != nil { player.stop() }
        titleLabel.text = request.title
        subtitleLabel.text = request.subtitle
        subtitleLabel.isHidden = (request.subtitle ?? "").isEmpty
        liveBadge.isHidden = !request.live
        [backButton, forwardButton, slider, timeLabel, durationLabel].forEach { $0.isHidden = request.live }
        messageLabel.isHidden = true
        spinner.startAnimating()

        let media = VLCMedia(url: url)
        media.addOption(":network-caching=1500")
        if request.start_seconds > 1 {
            media.addOption(":start-time=\(Int(request.start_seconds))")
        }
        player.media = media
        player.play()
        showOverlay()
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
        case .error:
            spinner.stopAnimating()
            showMessage("No se pudo reproducir. Puede que la cuenta esté en uso en otro dispositivo.")
            sendEvent(["type": "error", "message": "VLC no pudo abrir el stream"])
            self.request = nil
            return
        case .ended:
            if !request.live {
                close(ended: true)
                return
            }
        default:
            spinner.stopAnimating()
        }

        playButton.setImage(UIImage(systemName: player.isPlaying ? "pause.fill" : "play.fill"), for: .normal)

        if !request.live {
            let position = positionSeconds
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
        sendEvent(["type": "closed", "position_s": position, "duration_s": duration, "ended": ended])
        dismiss(animated: true)
    }

    // MARK: - Acciones

    @objc private func closeTapped() { close(ended: false) }

    @objc private func playTapped() {
        if player.isPlaying { player.pause() } else { player.play() }
        showOverlay()
    }

    @objc private func backTapped() { seek(by: -10) }
    @objc private func forwardTapped() { seek(by: 30) }

    private func seek(by seconds: Double) {
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
        guard request?.live == true else { return }
        // Deslizar hacia arriba pasa al canal siguiente, como en un mando.
        sendEvent(["type": "zap", "direction": gesture.direction == .up ? 1 : -1])
    }

    @objc private func willResignActive() {
        if player.isPlaying { player.pause() }
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
        view.addSubview(overlay)

        let config = UIImage.SymbolConfiguration(pointSize: 22, weight: .semibold)
        let bigConfig = UIImage.SymbolConfiguration(pointSize: 40, weight: .semibold)
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

        let topBar = UIStackView(arrangedSubviews: [closeButton, titles, liveBadge, audioButton, subtitlesButton])
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
        view.addSubview(spinner)

        messageLabel.textColor = .white
        messageLabel.numberOfLines = 0
        messageLabel.textAlignment = .center
        messageLabel.isHidden = true
        messageLabel.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(messageLabel)

        let guide = view.safeAreaLayoutGuide
        NSLayoutConstraint.activate([
            overlay.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            overlay.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            overlay.topAnchor.constraint(equalTo: view.topAnchor),
            overlay.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            topBar.leadingAnchor.constraint(equalTo: guide.leadingAnchor, constant: 16),
            topBar.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -16),
            topBar.topAnchor.constraint(equalTo: guide.topAnchor, constant: 12),
            center.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            center.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            bottomBar.leadingAnchor.constraint(equalTo: guide.leadingAnchor, constant: 16),
            bottomBar.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -16),
            bottomBar.bottomAnchor.constraint(equalTo: guide.bottomAnchor, constant: -12),
            spinner.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            spinner.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            messageLabel.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            messageLabel.centerYAnchor.constraint(equalTo: view.centerYAnchor, constant: 60),
            messageLabel.leadingAnchor.constraint(greaterThanOrEqualTo: guide.leadingAnchor, constant: 32),
            messageLabel.trailingAnchor.constraint(lessThanOrEqualTo: guide.trailingAnchor, constant: -32),
        ])
        titles.setContentHuggingPriority(.defaultLow, for: .horizontal)
    }

    private func showMessage(_ text: String) {
        messageLabel.text = text
        messageLabel.isHidden = false
        showOverlay(autoHide: false)
    }

    @objc private func toggleOverlay() {
        if overlay.alpha > 0.5 { hideOverlay() } else { showOverlay() }
    }

    private func showOverlay(autoHide: Bool = true) {
        UIView.animate(withDuration: 0.2) { self.overlay.alpha = 1 }
        hideTimer?.invalidate()
        guard autoHide else { return }
        hideTimer = Timer.scheduledTimer(withTimeInterval: 4, repeats: false) { [weak self] _ in
            guard let self = self, self.player.isPlaying, !self.scrubbing else { return }
            self.hideOverlay()
        }
    }

    private func hideOverlay() {
        UIView.animate(withDuration: 0.3) { self.overlay.alpha = 0 }
    }

    private func formatTime(_ seconds: Double) -> String {
        let total = Int(max(0, seconds))
        let h = total / 3600, m = (total % 3600) / 60, s = total % 60
        return h > 0 ? String(format: "%d:%02d:%02d", h, m, s) : String(format: "%d:%02d", m, s)
    }
}
