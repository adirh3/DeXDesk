use crate::{
    advertise::Advertisement,
    media::NativeVideo,
    protocol::{self, RtpReorder, RtspFramer, VideoMode},
    runtime::{AppState, Phase, VIDEO_STARTED, VIDEO_STOPPED},
    settings::Quality,
    transport_stream::TransportStream,
};
use anyhow::{bail, Context, Result};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    io::{Read, Write},
    net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream, UdpSocket},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
use windows::{
    Devices::WiFiDirect::*,
    Foundation::{AsyncStatus, TypedEventHandler},
    Win32::{
        Foundation::HWND,
        System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
    },
};

struct Apartment;
impl Apartment {
    fn new() -> Result<Self> {
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}

pub fn start(state: Arc<AppState>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let result = run(&state);
        if let Err(error) = result {
            state.log(format!("Receiver stopped: {error:#}"));
            if state.running.load(Ordering::Acquire) {
                state.set_phase(Phase::ReceiverUnavailable(error.to_string()));
            }
        }
    })
}

fn run(state: &Arc<AppState>) -> Result<()> {
    let _apartment = Apartment::new()?;
    let guard = TcpListener::bind(("0.0.0.0", 7236)).context(
        "Could not open Miracast port 7236. Close Windows Wireless Display or any other receiver, then retry.",
    )?;
    guard.set_nonblocking(true)?;
    let (sender, requests) = mpsc::sync_channel(1);
    let mut advertisement = Advertisement::new(state.clone())?;
    let callback_state = state.clone();
    let request_pending = Arc::new(AtomicBool::new(false));
    let callback_pending = request_pending.clone();
    let token = advertisement
        .listener
        .ConnectionRequested(&TypedEventHandler::new(
            move |_: &Option<WiFiDirectConnectionListener>,
                  args: &Option<WiFiDirectConnectionRequestedEventArgs>| {
                let Some(args) = args else {
                    callback_state.log("Windows delivered an empty Wi-Fi Direct request");
                    return Ok(());
                };
                if !callback_state.running.load(Ordering::Acquire) {
                    return Ok(());
                }
                if callback_pending.swap(true, Ordering::AcqRel) {
                    callback_state.log(
                        "Ignored another connection request while a phone is already connected",
                    );
                    return Ok(());
                }
                match args.GetConnectionRequest() {
                    Ok(request) => {
                        if sender.try_send(request).is_err() {
                            callback_pending.store(false, Ordering::Release);
                            callback_state.log("Could not queue the Wi-Fi Direct request");
                        }
                    }
                    Err(error) => {
                        callback_pending.store(false, Ordering::Release);
                        callback_state
                            .log(format!("Windows rejected the connection request: {error}"));
                    }
                }
                Ok(())
            },
        ))?;
    let result = (|| -> Result<()> {
        advertisement.start()?;
        receive_requests(state, &guard, &requests, &request_pending)
    })();
    if let Err(error) = advertisement.listener.RemoveConnectionRequested(token) {
        state.log(format!("Could not unregister Wi-Fi Direct requests: {error}"));
    }
    result
}

fn receive_requests(
    state: &Arc<AppState>,
    guard: &TcpListener,
    requests: &mpsc::Receiver<WiFiDirectConnectionRequest>,
    request_pending: &AtomicBool,
) -> Result<()> {
    state.set_phase(Phase::Ready);
    while state.running.load(Ordering::Acquire) {
        let request = match requests.recv_timeout(Duration::from_millis(150)) {
            Ok(value) => value,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(error) => return Err(error.into()),
        };
        state.disconnect.store(false, Ordering::Release);
        state.session_active.store(true, Ordering::Release);
        let result = connect_phone(state, guard, request);
        state.live.store(false, Ordering::Release);
        *state.input.lock().unwrap() = None;
        *state.mode.lock().unwrap() = None;
        state.frames.store(0, Ordering::Release);
        state.session_active.store(false, Ordering::Release);
        request_pending.store(false, Ordering::Release);
        state.notify(VIDEO_STOPPED);
        if !state.running.load(Ordering::Acquire) {
            break;
        }
        match result {
            _ if state.disconnect.load(Ordering::Acquire) => state.set_phase(Phase::Ready),
            Ok(()) => state.set_phase(Phase::Ready),
            Err(error) => {
                state.log(format!("Connection ended: {error:#}"));
                state.set_phase(Phase::Error(error.to_string()));
            }
        }
    }
    Ok(())
}

fn connect_phone(
    state: &Arc<AppState>,
    guard: &TcpListener,
    request: WiFiDirectConnectionRequest,
) -> Result<()> {
    let information = request.DeviceInformation()?;
    state.set_phase(Phase::Connecting(information.Name()?.to_string()));
    let parameters = WiFiDirectConnectionParameters::new()?;
    parameters.SetGroupOwnerIntent(15)?;
    let operation = WiFiDirectDevice::FromIdAsync2(&information.Id()?, &parameters)?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while operation.Status()? == AsyncStatus::Started {
        if stopping(state) || Instant::now() > deadline {
            operation.Cancel()?;
            bail!("The Wi-Fi Direct connection was cancelled or timed out");
        }
        thread::sleep(Duration::from_millis(30));
    }
    let device = operation
        .GetResults()
        .context("Windows could not connect to the phone")?;
    let result = (|| -> Result<()> {
        let endpoints = device.GetConnectionEndpointPairs()?;
        let mut peer = None;
        for endpoint in endpoints {
            let local = endpoint
                .LocalHostName()?
                .CanonicalName()?
                .to_string()
                .parse::<IpAddr>();
            let remote = endpoint
                .RemoteHostName()?
                .CanonicalName()?
                .to_string()
                .parse::<IpAddr>();
            if let (Ok(IpAddr::V4(local)), Ok(IpAddr::V4(remote))) = (local, remote) {
                peer = Some((IpAddr::V4(local), IpAddr::V4(remote)));
                break;
            }
        }
        let (local, remote) =
            peer.context("Windows did not provide IPv4 Wi-Fi Direct endpoints")?;
        state.set_phase(Phase::Negotiating);
        state.log(format!("Wi-Fi Direct connected: {local} <-> {remote}"));
        let stream = match tcp(local, remote, 7236) {
            Ok(stream) => stream,
            Err(error) => {
                state.log(format!(
                    "Phone RTSP dial failed ({error}); waiting for an inbound session"
                ));
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    if stopping(state) || Instant::now() > deadline {
                        bail!("The phone did not start a Miracast session");
                    }
                    match guard.accept(){
                        Ok((stream,address)) if address.ip()==remote=>break stream,
                        Ok(_)=>state.log("Rejected an RTSP connection from outside the connected Wi-Fi Direct peer"),
                        Err(error) if error.kind()==std::io::ErrorKind::WouldBlock=>thread::sleep(Duration::from_millis(30)),
                        Err(error)=>return Err(error.into()),
                    }
                }
            }
        };
        session(state, stream, local, remote)
    })();
    if let Err(error) = device.Close() {
        state.log(format!("Could not close the Wi-Fi Direct device: {error}"));
    }
    result
}

fn stopping(state: &AppState) -> bool {
    !state.running.load(Ordering::Acquire) || state.disconnect.load(Ordering::Acquire)
}

fn tcp(local: IpAddr, remote: IpAddr, port: u16) -> Result<TcpStream> {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    socket.bind(&SocketAddr::new(local, 0).into())?;
    socket.connect_timeout(
        &SocketAddr::new(remote, port).into(),
        Duration::from_secs(5),
    )?;
    let stream: TcpStream = socket.into();
    configure_tcp(&stream)?;
    Ok(stream)
}

fn configure_tcp(stream: &TcpStream) -> Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_millis(120)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    Ok(())
}

fn udp_pair(local: IpAddr, state: &AppState) -> Result<(UdpSocket, UdpSocket)> {
    for _ in 0..24 {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_recv_buffer_size(4 * 1024 * 1024)?;
        socket.bind(&SocketAddr::new(local, 0).into())?;
        let first: UdpSocket = socket.into();
        let port = first.local_addr()?.port();
        if port % 2 != 0 {
            continue;
        }
        match UdpSocket::bind(SocketAddr::new(local, port + 1)) {
            Ok(second) => {
                first.set_read_timeout(Some(Duration::from_millis(20)))?;
                second.set_nonblocking(true)?;
                return Ok((first, second));
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                state.log(format!(
                    "Could not reserve the paired RTCP port: {error}; retrying an OS-selected pair"
                ));
            }
            Err(error) => return Err(error.into()),
        }
    }
    bail!("Windows could not reserve an RTP/RTCP port pair")
}

fn capabilities(request: &str, port: u16, quality: Quality) -> String {
    let values=[
        ("wfd_video_formats",quality.formats().to_owned()),
        ("wfd_audio_codecs","LPCM 00000002 00".into()),
        ("wfd_client_rtp_ports",format!("RTP/AVP/UDP;unicast {port} 0 mode=play")),
        ("wfd_content_protection","none".into()),
        ("wfd_display_edid","none".into()),
        ("wfd_coupled_sink","none".into()),
        ("wfd_connector_type","05".into()),
        ("wfd_idr_request_capability","1".into()),
        ("wfd_uibc_capability","input_category_list=GENERIC, HIDC;generic_cap_list=Mouse, SingleTouch, MultiTouch;hidc_cap_list=Keyboard/USB;port=none".into()),
    ];
    let mut body = String::new();
    for name in request
        .lines()
        .map(|line| line.split(':').next().unwrap_or("").trim())
        .filter(|name| !name.is_empty())
    {
        let value = values
            .iter()
            .find(|(key, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
            .unwrap_or("none");
        body.push_str(&format!("{name}: {value}\r\n"));
    }
    body
}

fn write(stream: &mut TcpStream, message: String) -> Result<()> {
    stream
        .write_all(message.as_bytes())
        .context("Miracast control connection failed")
}

const RTSP_START_TIMEOUT: Duration = Duration::from_secs(20);
const FIRST_PICTURE_TIMEOUT: Duration = Duration::from_secs(15);

fn check_session_start(elapsed: Duration, input_ready: bool, picture_ready: bool) -> Result<()> {
    if !input_ready && elapsed >= RTSP_START_TIMEOUT {
        bail!("The phone did not finish setting up Miracast. End the cast on your Galaxy and try again.");
    }
    if input_ready && !picture_ready && elapsed >= FIRST_PICTURE_TIMEOUT {
        bail!("The phone connected, but no picture arrived. End the cast and try again.");
    }
    Ok(())
}

fn session(
    state: &Arc<AppState>,
    mut stream: TcpStream,
    local: IpAddr,
    remote: IpAddr,
) -> Result<()> {
    configure_tcp(&stream)?;
    let (rtp, rtcp) = udp_pair(local, state)?;
    let rtp_port = rtp.local_addr()?.port();
    let mut udp = Some((rtp, rtcp));
    let life = Arc::new(AtomicBool::new(true));
    let recover = Arc::new(AtomicBool::new(false));
    let mut video_thread = None;
    let mut input_thread = None;
    let mut input_stream = None;
    let mut mode: Option<VideoMode> = None;
    let mut sequence = 0;
    let mut setup_sequence = None;
    let mut play_sequence = None;
    let mut session_id = String::new();
    let mut url = format!("rtsp://{remote}/wfd1.0/streamid=0");
    let mut uibc = String::new();
    let mut options = false;
    let mut framer = RtspFramer::default();
    let mut bytes = [0u8; 8192];
    let mut last_recover = Instant::now() - Duration::from_secs(2);
    let mut stage_started = Instant::now();
    let quality = state.settings.lock().unwrap().quality;
    state.log(format!(
        "Starting Miracast; quality target: {}",
        quality.label()
    ));

    let result = (|| -> Result<()> {
        while !stopping(state) && life.load(Ordering::Acquire) {
            if input_thread.is_some()
                && !state.live.load(Ordering::Acquire)
                && state.frames.load(Ordering::Acquire) > 0
            {
                state.live.store(true, Ordering::Release);
                state.notify(VIDEO_STARTED);
            }
            check_session_start(
                stage_started.elapsed(),
                input_thread.is_some(),
                state.frames.load(Ordering::Acquire) > 0,
            )?;
            if recover.load(Ordering::Acquire)
                && last_recover.elapsed() > Duration::from_secs(1)
                && !session_id.is_empty()
            {
                recover.store(false, Ordering::Release);
                sequence += 1;
                write(
                    &mut stream,
                    protocol::rtsp(
                        &format!("SET_PARAMETER {url} RTSP/1.0"),
                        sequence,
                        "wfd_idr_request\r\n",
                        &format!("Session: {session_id}\r\n"),
                    ),
                )?;
                last_recover = Instant::now();
            }
            match stream.read(&mut bytes) {
                Ok(0) if state.live.load(Ordering::Acquire) => break,
                Ok(0) => bail!("The phone closed the connection before sending a picture"),
                Ok(count) => framer.push(&bytes[..count])?,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue
                }
                Err(error) => return Err(error.into()),
            }
            while let Some(message) = framer.next_message()? {
                if stopping(state) || !life.load(Ordering::Acquire) {
                    return Ok(());
                }
                check_session_start(
                    stage_started.elapsed(),
                    input_thread.is_some(),
                    state.frames.load(Ordering::Acquire) > 0,
                )?;
                let received = message.sequence()?;
                if message.response() {
                    if !message.start.starts_with("RTSP/1.0 200") {
                        bail!("The phone rejected a control request: {}", message.start);
                    }
                    if setup_sequence == Some(received) {
                        setup_sequence = None;
                        session_id = message
                            .headers
                            .get("session")
                            .context("Missing Miracast session identifier")?
                            .split(';')
                            .next()
                            .unwrap_or("")
                            .to_owned();
                        sequence += 1;
                        play_sequence = Some(sequence);
                        write(
                            &mut stream,
                            protocol::rtsp(
                                &format!("PLAY {url} RTSP/1.0"),
                                sequence,
                                "",
                                &format!("Session: {session_id}\r\n"),
                            ),
                        )?;
                    } else if play_sequence == Some(received) {
                        play_sequence = None;
                        let selected = mode.context("The phone did not select a video format")?;
                        let (port, pointer, keyboard) = protocol::input_capability(&uibc);
                        if port == 0 || !pointer || !keyboard {
                            bail!("This phone did not allow the required mouse/keyboard input channel");
                        }
                        sequence += 1;
                        write(
                            &mut stream,
                            protocol::rtsp(
                                &format!("SET_PARAMETER {url} RTSP/1.0"),
                                sequence,
                                "wfd_uibc_setting: enable\r\n",
                                &format!("Session: {session_id}\r\n"),
                            ),
                        )?;
                        let mut input = tcp(local, remote, port)?;
                        input.write_all(&protocol::keyboard_descriptor())?;
                        input_stream = Some(input.try_clone()?);
                        let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(512);
                        *state.input.lock().unwrap() = Some(sender);
                        let worker_state = state.clone();
                        let worker_life = life.clone();
                        input_thread = Some(thread::spawn(move || {
                            while worker_life.load(Ordering::Acquire) && !stopping(&worker_state) {
                                match receiver.recv_timeout(Duration::from_millis(100)) {
                                    Ok(packet) => {
                                        if let Err(error) = input.write_all(&packet) {
                                            worker_state
                                                .log(format!("Input connection failed: {error}"));
                                            worker_life.store(false, Ordering::Release);
                                            break;
                                        }
                                    }
                                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                                    Err(_) => break,
                                }
                            }
                            let _ = input.shutdown(Shutdown::Both);
                        }));
                        state.log(format!("Session ready: {}x{} at {} FPS, generic pointer and native HID keyboard",selected.width,selected.height,selected.fps));
                        stage_started = Instant::now();
                        state.set_phase(Phase::WaitingForVideo);
                        recover.store(true, Ordering::Release);
                    }
                    continue;
                }
                match message.method() {
                    "OPTIONS" => {
                        write(&mut stream,protocol::rtsp("RTSP/1.0 200 OK",received,"","Public: org.wfa.wfd1.0, OPTIONS, GET_PARAMETER, SET_PARAMETER, SETUP, PLAY, TEARDOWN\r\n"))?;
                        if !options {
                            options = true;
                            sequence += 1;
                            write(
                                &mut stream,
                                protocol::rtsp(
                                    "OPTIONS * RTSP/1.0",
                                    sequence,
                                    "",
                                    "Require: org.wfa.wfd1.0\r\n",
                                ),
                            )?;
                        }
                    }
                    "GET_PARAMETER" => write(
                        &mut stream,
                        protocol::rtsp(
                            "RTSP/1.0 200 OK",
                            received,
                            &if message.body.is_empty() {
                                String::new()
                            } else {
                                capabilities(&message.body, rtp_port, quality)
                            },
                            "",
                        ),
                    )?,
                    "SET_PARAMETER" => {
                        let parameters = protocol::parameters(&message.body);
                        if let Some(value) = parameters.get("wfd_video_formats") {
                            let selected = VideoMode::from_wfd(value)?;
                            if mode.is_some_and(|old| old != selected) {
                                bail!(
                                    "The phone changed video mode; reconnect to apply the new mode"
                                );
                            }
                            mode = Some(selected);
                            *state.mode.lock().unwrap() = mode;
                            state.log(format!(
                                "Phone selected {}x{} / {} FPS",
                                selected.width, selected.height, selected.fps
                            ));
                            if let Some((rtp, rtcp)) = udp.take() {
                                video_thread = Some(video_worker(
                                    state.clone(),
                                    rtp,
                                    rtcp,
                                    remote,
                                    selected,
                                    life.clone(),
                                    recover.clone(),
                                ));
                            }
                        }
                        if let Some(value) = parameters.get("wfd_uibc_capability") {
                            uibc = value.clone();
                        }
                        if let Some(value) = parameters.get("wfd_presentation_url") {
                            url = value
                                .split_whitespace()
                                .next()
                                .context("Empty phone presentation URL")?
                                .to_owned();
                        }
                        write(
                            &mut stream,
                            protocol::rtsp("RTSP/1.0 200 OK", received, "", ""),
                        )?;
                        match parameters.get("wfd_trigger_method").map(String::as_str) {
                            Some("SETUP") if setup_sequence.is_none()
                                && play_sequence.is_none()
                                && input_thread.is_none() =>
                            {
                                sequence += 1;
                                setup_sequence = Some(sequence);
                                write(&mut stream,protocol::rtsp(&format!("SETUP {url} RTSP/1.0"),sequence,"",
                                    &format!("Transport: RTP/AVP/UDP;unicast;client_port={rtp_port}-{}\r\n",rtp_port+1)))?;
                            }
                            Some("TEARDOWN") => return Ok(()),
                            _ => {}
                        }
                    }
                    "TEARDOWN" => {
                        write(
                            &mut stream,
                            protocol::rtsp("RTSP/1.0 200 OK", received, "", ""),
                        )?;
                        return Ok(());
                    }
                    _ => write(
                        &mut stream,
                        protocol::rtsp("RTSP/1.0 405 Method Not Allowed", received, "", ""),
                    )?,
                }
            }
        }
        Ok(())
    })();
    life.store(false, Ordering::Release);
    *state.input.lock().unwrap() = None;
    if let Some(input) = input_stream {
        let _ = input.shutdown(Shutdown::Both);
    }
    let _ = stream.shutdown(Shutdown::Both);
    if let Some(worker) = input_thread {
        if worker.join().is_err() {
            state.log("Input worker stopped unexpectedly");
        }
    }
    if let Some(worker) = video_thread {
        match worker.join() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(error),
            Err(_) => bail!("Windows video worker stopped unexpectedly"),
        }
    }
    result
}

fn video_worker(
    state: Arc<AppState>,
    rtp: UdpSocket,
    rtcp: UdpSocket,
    remote: IpAddr,
    mode: VideoMode,
    life: Arc<AtomicBool>,
    recover: Arc<AtomicBool>,
) -> thread::JoinHandle<Result<()>> {
    thread::spawn(move || {
        let result = (|| -> Result<()> {
            let _apartment = Apartment::new()?;
            let window = HWND(state.video_hwnd.load(Ordering::Acquire) as *mut _);
            let mut video = NativeVideo::new(window, mode)?;
            *state.backend.lock().unwrap() = if video.hardware {
                "Windows H.264 / DXGI / Direct3D 11"
            } else {
                "Windows H.264 / CPU upload / Direct3D 11"
            }
            .into();
            let mut demux = TransportStream::default();
            let mut reorder = RtpReorder::default();
            let mut packet = [0u8; 65536];
            let started = Instant::now();
            while life.load(Ordering::Acquire) && !stopping(&state) {
                let batch = match rtp.recv_from(&mut packet) {
                    Ok((length, peer)) if peer.ip() == remote => {
                        let Some((source, sequence, payload)) =
                            protocol::rtp_payload(&packet[..length])
                        else {
                            continue;
                        };
                        reorder.push(
                            source,
                            sequence,
                            payload,
                            started.elapsed().as_millis() as u64,
                        )
                    }
                    Ok(_) => continue,
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        reorder.flush(started.elapsed().as_millis() as u64)
                    }
                    Err(error) => return Err(error.into()),
                };
                if batch.1 > 0 {
                    state.log(format!(
                        "Video recovery: {} missing RTP sequence numbers",
                        batch.1
                    ));
                    demux.discard();
                    recover.store(true, Ordering::Release);
                }
                for payload in batch.0 {
                    for picture in demux.push(&payload)? {
                        let count = video.push(&picture.bytes, picture.discontinuity)?;
                        state.frames.store(count, Ordering::Release);
                    }
                }
                let mut control = [0u8; 1500];
                for _ in 0..32 {
                    if rtcp.recv_from(&mut control).is_err() {
                        break;
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = &result {
            state.log(format!("Native video failed: {error:#}"));
            life.store(false, Ordering::Release);
        }
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiation_times_out_even_if_video_started_early() {
        assert!(check_session_start(
            RTSP_START_TIMEOUT - Duration::from_millis(1),
            false,
            false
        )
        .is_ok());
        assert!(check_session_start(RTSP_START_TIMEOUT, false, true).is_err());
    }

    #[test]
    fn first_picture_has_a_separate_deadline() {
        assert!(check_session_start(
            FIRST_PICTURE_TIMEOUT - Duration::from_millis(1),
            true,
            false
        )
        .is_ok());
        assert!(check_session_start(FIRST_PICTURE_TIMEOUT, true, false).is_err());
        assert!(check_session_start(Duration::from_secs(60), true, true).is_ok());
    }
}
