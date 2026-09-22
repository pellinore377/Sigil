//! Silent synthetic acceptance: native AV1 RTP through the production encrypted transforms.
//! Built only with video-acceptance; never captures a device or creates an audio track.
use crate::{
    fail, get,
    rtc::{construct, invoke, object},
    set,
};
use js_sys::{Array, Function};
#[derive(Default)]
struct Presented {
    frames: u64,
    last: f64,
    gap: f64,
}
thread_local! {static PRESENTED:RefCell<std::collections::HashMap<String,Presented>>=RefCell::new(std::collections::HashMap::new());}
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::{JsFuture, spawn_local};
async fn promise(v: JsValue) -> Result<JsValue, JsValue> {
    JsFuture::from(v.dyn_into::<js_sys::Promise>()?).await
}
pub(crate) async fn receive_burst(options: &JsValue, frame: u32) -> Result<(), JsValue> {
    if frame % 600 != 0 || get(options, "receive_bursts").ok().and_then(|v| v.as_bool()) != Some(true) { return Ok(()); }
    let pause = js_sys::Promise::new(&mut |resolve, reject| {
        let worker: web_sys::WorkerGlobalScope = js_sys::global().unchecked_into();
        if let Err(error) = worker.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 150) {
            let _ = reject.call1(&JsValue::UNDEFINED, &error);
        }
    });
    promise(pause.into()).await?;
    crate::transform::timing(format!("SigilTiming acceptance receive burst frame={frame}"));
    Ok(())
}
fn worker() -> Result<web_sys::Worker, JsValue> {
    let options = web_sys::WorkerOptions::new();
    options.set_type(web_sys::WorkerType::Module);
    web_sys::Worker::new_with_options("/web/sigil-media-worker.mjs", &options)
}
fn attach(
    worker: &web_sys::Worker,
    target: &JsValue,
    call: &str,
    sender: Option<&str>,
) -> Result<(), JsValue> {
    if web_sys::window()
        .and_then(|w| w.location().search().ok())
        .is_some_and(|s| s == "?plain")
    {
        return Ok(());
    }
    let options = object(
        serde_json::json!({"operation":if sender.is_some(){"open"}else{"seal"},"kind":"camera","call":call,"sender":sender.unwrap_or("")}),
    )?;
    set(&options, "drop_bursts", &web_sys::window().unwrap().location().search()?.eq("?recovery").into())?;
    set(&options, "receive_bursts", &web_sys::window().unwrap().location().search()?.contains("burst").into())?;
    let constructor = get(&js_sys::global(), "RTCRtpScriptTransform")?.dyn_into::<Function>()?;
    let value = js_sys::Reflect::construct(&constructor, &Array::of2(worker, &options))?;
    set(target, "transform", &value)
}
fn media_worker(update: serde_json::Value) -> Result<(web_sys::Worker, web_sys::Worker), JsValue> {
    let media = worker()?;
    let reply = media.clone();
    let gate = js_sys::SharedArrayBuffer::new(12);
    let presentation_gate = js_sys::Int32Array::new(&gate);
    let events =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            if let Ok(frame) = get(&e.data(), "frame") {
                if !frame.is_undefined() {
                    let data = e.data();
                    let sender = get(&data, "sender").unwrap().as_string().unwrap();
                    let rotation = get(&data, "rotation").unwrap().as_f64().unwrap() as u16;
                    let token = get(&data, "id").unwrap().as_f64().unwrap() as u32;
                    let valid = js_sys::Atomics::load(&presentation_gate, 0).ok().map(f64::from) == get(&data, "revision").ok().and_then(|v| v.as_f64())
                        && get(&data, "until").ok().and_then(|v| v.as_f64()).is_some_and(|until| js_sys::Date::now() < until);
                    if valid {
                    if let Err(error)=crate::video::native_frame(&sender, &frame, rotation, token) { web_sys::console::error_1(&error); }
                    PRESENTED.with(|p| {
                        let mut p = p.borrow_mut();
                        let v = p.entry(sender).or_default();
                        let now = js_sys::Date::now();
                        if v.last > 0.0 {
                            v.gap = v.gap.max(now - v.last);
                        }
                        v.last = now;
                        v.frames += 1;
                    });
                    }
                    let _ = set(&data, "video_ack", &true.into());
                    let pixels = get(&frame,"pixels").unwrap().dyn_into::<js_sys::Uint8Array>().unwrap();
                    let _ = reply.post_message_with_transfer(&data,&Array::of1(&pixels.buffer()));
                }
            }
            if let Some(v) = get(&e.data(), "media_timing").ok().and_then(|v| v.as_string()) {
                web_sys::console::log_1(&v.into());
            }
        });
    media.set_onmessage(Some(events.as_ref().unchecked_ref()));
    events.forget();
    let channel = web_sys::MessageChannel::new()?;
    let options = web_sys::WorkerOptions::new();
    options.set_type(web_sys::WorkerType::Module);
    let control = web_sys::Worker::new_with_options("/web/sigil-video-control.mjs", &options)?;
    let diagnostics = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(|event: web_sys::MessageEvent| {
        if let Some(message) = get(&event.data(), "media_timing").ok().and_then(|v| v.as_string()) {
            if let Some(checks) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id("checks")) {
                let previous = checks.text_content().unwrap_or_default();
                checks.set_text_content(Some(&format!("{previous}\n{message}")));
            }
            web_sys::console::log_1(&message.into());
        }
    });
    control.set_onmessage(Some(diagnostics.as_ref().unchecked_ref()));
    diagnostics.forget();
    let init = js_sys::Object::new();
    set(&init, "module", &wasm_bindgen::module())?;
    set(&init, "gate", &gate)?;
    set(&init, "port", &channel.port2())?;
    media.post_message_with_transfer(&init, &Array::of1(&channel.port2()))?;
    set(&init, "port", &channel.port1())?;
    set(&init, "fixture", &update.to_string().into())?;
    set(&init, "revoke", &web_sys::window().unwrap().location().search()?.contains("revoke").into())?;
    control.post_message_with_transfer(&init, &Array::of1(&channel.port1()))?;
    Ok((media, control))
}
#[wasm_bindgen]
pub fn video_test_control_start(
    port: web_sys::MessagePort,
    buffer: JsValue,
    fixture: String,
    revoke: bool,
) -> Result<(), JsValue> {
    let update = Rc::new(RefCell::new(
        serde_json::from_str::<serde_json::Value>(&fixture).map_err(|_| fail("Fixture"))?,
    ));
    let gate = js_sys::Int32Array::new(&buffer);
    crate::media_worker::test_control_link(port, gate.clone());
    let publish = |update: &serde_json::Value| -> Result<(), JsValue> {
        let update = serde_json::from_value(update.clone()).map_err(|_| fail("Fixture update"))?;
        crate::media_worker::publish(update)
    };
    publish(&update.borrow())?;
    update.borrow_mut()["sender"] = serde_json::Value::Null;
    let pending = update.clone();
    let changed =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            if let Some(raw) = event.data().as_string() {
                if let Ok(next) = serde_json::from_str(&raw) {
                    *pending.borrow_mut() = next;
                }
            }
        });
    js_sys::global()
        .unchecked_into::<web_sys::DedicatedWorkerGlobalScope>()
        .set_onmessage(Some(changed.as_ref().unchecked_ref()));
    changed.forget();
    let tick = Cell::new(0);
    let timer = Closure::<dyn FnMut()>::new(move || {
        let _ = publish(&update.borrow());
        update.borrow_mut()["sender"] = serde_json::Value::Null;
        tick.set(tick.get() + 1);
        // Same synchronous wait used by the store's HTTP bridge, deliberately worse than normal.
        if tick.get() % 20 == 0 {
            let wait = js_sys::Int32Array::new(&js_sys::SharedArrayBuffer::new(4));
            if revoke && tick.get() == 20 {
                crate::media_worker::invalidate();
                let _ = crate::transport::wait_for_response(&wait, 100.0);
                let before = (js_sys::Atomics::load(&gate, 1).unwrap(), js_sys::Atomics::load(&gate, 2).unwrap());
                let _ = crate::transport::wait_for_response(&wait, 2900.0);
                let after = (js_sys::Atomics::load(&gate, 1).unwrap(), js_sys::Atomics::load(&gate, 2).unwrap());
                assert_eq!(before, after, "network heartbeat revived revoked media");
                crate::transform::timing("SigilTiming acceptance revoked media stayed stopped during network wait".into());
            } else if revoke && tick.get() == 40 {
                let _ = js_sys::Atomics::wait_with_timeout(&wait, 0, 0, 2200.0);
                let before = (js_sys::Atomics::load(&gate, 1).unwrap(), js_sys::Atomics::load(&gate, 2).unwrap());
                let _ = js_sys::Atomics::wait_with_timeout(&wait, 0, 0, 300.0);
                let after = (js_sys::Atomics::load(&gate, 1).unwrap(), js_sys::Atomics::load(&gate, 2).unwrap());
                assert_eq!(before, after, "stopped control worker retained media authority");
                crate::transform::timing("SigilTiming acceptance stopped worker expired media authority".into());
            } else {
                let _ = crate::transport::wait_for_response(&wait, 3000.0);
            }
        }
    });
    js_sys::global()
        .unchecked_into::<web_sys::DedicatedWorkerGlobalScope>()
        .set_interval_with_callback_and_timeout_and_arguments_0(
            timer.as_ref().unchecked_ref(),
            100,
        )?;
    timer.forget();
    Ok(())
}
struct Run {
    pcs: Vec<JsValue>,
    workers: Vec<web_sys::Worker>,
    timers: Vec<i32>,
    tracks: Vec<JsValue>,
    handlers: Vec<Closure<dyn FnMut(JsValue)>>,
    callbacks: Vec<Closure<dyn FnMut()>>,
    running: Rc<Cell<bool>>,
}
thread_local! {static RUN:RefCell<Option<Run>>=const{RefCell::new(None)};}
#[wasm_bindgen]
pub fn video_test_stop() {
    RUN.with(|r| {
        if let Some(run) = r.borrow_mut().take() {
            run.running.set(false);
            for id in run.timers {
                web_sys::window().unwrap().clear_interval_with_handle(id);
            }
            for pc in run.pcs {
                let _ = invoke(&pc, "close", &[]);
            }
            for t in run.tracks {
                let _ = invoke(&t, "stop", &[]);
            }
            for w in run.workers {
                w.terminate();
            }
            drop(run.handlers);
            drop(run.callbacks);
        }
    });
}
#[wasm_bindgen]
pub async fn video_test_start(width: u32, height: u32, fps: u32) -> Result<(), JsValue> {
    video_test_stop();
    PRESENTED.with(|p| p.borrow_mut().clear());
    let document = web_sys::window().unwrap().document().unwrap();
    let body = document.body().unwrap();
    body.set_inner_html("<h1>Silent encrypted AV1 test</h1><p>Two synthetic video tracks; no microphone or speakers. Control workers wait three seconds for network replies every two seconds.</p><pre id='checks'></pre><pre id='stats' style='max-height:220px;overflow:auto'>Starting</pre>");
    let call = [1u8; 32];
    let members = [[3u8; 32], [4u8; 32]];
    let mut workers = Vec::new();
    for update in fixture_updates() {
        let (media, control) = media_worker(update)?;
        workers.push(media);
        workers.push(control);
    }
    let running = Rc::new(Cell::new(true));
    let mut timers = Vec::new();
    let mut callbacks = Vec::new();
    let mut handlers = Vec::new();
    let controls = [workers[1].clone(), workers[3].clone()];
    let rotate = Closure::<dyn FnMut()>::new(move || {
        for (worker, update) in controls.iter().zip(fixture_updates()) {
            let _ = worker.post_message(&update.to_string().into());
        }
    });
    timers.push(
        web_sys::window()
            .unwrap()
            .set_interval_with_callback_and_timeout_and_arguments_0(
                rotate.as_ref().unchecked_ref(),
                120_000,
            )?,
    );
    callbacks.push(rotate);
    let mut captured = Vec::new();
    let pcs = vec![
        construct(
            "RTCPeerConnection",
            &object(
                serde_json::json!({"encodedInsertableStreams":!web_sys::window().unwrap().location().search()?.eq("?plain")}),
            )?,
        )?,
        construct(
            "RTCPeerConnection",
            &object(
                serde_json::json!({"encodedInsertableStreams":!web_sys::window().unwrap().location().search()?.eq("?plain")}),
            )?,
        )?,
    ];
    let call_hex = crate::call::hex(call);
    for i in 0..2 {
        let canvas = document
            .create_element("canvas")?
            .dyn_into::<web_sys::HtmlCanvasElement>()?;
        canvas.set_width(width);
        canvas.set_height(height);
        canvas.set_id(&format!("source{i}"));
        canvas.set_attribute("style", "width:20%")?;
        body.append_child(&canvas)?;
        let context = canvas
            .get_context_with_context_options("2d", &object(serde_json::json!({"willReadFrequently":true}))?)?
            .unwrap()
            .dyn_into::<web_sys::CanvasRenderingContext2d>()?;
        let paint = move || {
            let t = js_sys::Date::now();
            context.set_fill_style_str(if i == 0 { "#103060" } else { "#601030" });
            context.fill_rect(0.0, 0.0, width as f64, height as f64);
            let x = (t / 5.0) % (width as f64);
            for row in 0..12 {
                for col in 0..20 {
                    let shift = (t / 20.0) % 96.0;
                    context.set_fill_style_str(&format!(
                        "hsl({},60%,{}%)",
                        (col * 17 + row * 29) % 360,
                        30 + (row + col) % 40
                    ));
                    context.fill_rect(
                        col as f64 * 96.0 - shift,
                        220.0 + row as f64 * 64.0,
                        86.0,
                        54.0,
                    );
                }
            }
            context.set_fill_style_str("#ffffff");
            context.fill_rect(x, 100.0, 50.0, height as f64 - 200.0);
            for bit in 0..24 {
                context.set_fill_style_str(if ((t as u64 >> bit) & 1) == 1 {
                    "#ffffff"
                } else {
                    "#000000"
                });
                context.fill_rect(
                    bit as f64 * width as f64 / 24.0,
                    0.0,
                    width as f64 / 24.0,
                    70.0,
                );
            }
            context.set_fill_style_str("#40e080");
            context.set_font("48px sans-serif");
            let _ = context.fill_text(
                &format!("Synthetic {width}×{height} {fps}fps  {}", t as u64),
                70.0,
                160.0,
            );
        };
        paint();
        let draw = Closure::<dyn FnMut()>::new(paint);
        timers.push(
            web_sys::window()
                .unwrap()
                .set_interval_with_callback_and_timeout_and_arguments_0(
                    draw.as_ref().unchecked_ref(),
                    (1000 / fps) as i32,
                )?,
        );
        callbacks.push(draw);
        let stream = invoke(&canvas, "captureStream", &[fps.into()])?;
        let preview = document
            .create_element("video")?
            .dyn_into::<web_sys::HtmlVideoElement>()?;
        preview.set_muted(true);
        preview.set_autoplay(true);
        preview.set_attribute("style", "width:20%")?;
        set(&preview, "srcObject", &stream)?;
        body.append_child(&preview)?;
        let track = Array::from(&invoke(&stream, "getVideoTracks", &[])?).get(0);
        captured.push(track.clone());
        invoke(&pcs[i], "addTrack", &[track])?;
        let transceiver = Array::from(&invoke(&pcs[i], "getTransceivers", &[])?).get(0);
        let capabilities = invoke(
            &get(&js_sys::global(), "RTCRtpSender")?,
            "getCapabilities",
            &["video".into()],
        )?;
        let codecs = Array::from(&get(&capabilities, "codecs")?);
        let av1 = Array::new();
        for codec in codecs.iter() {
            if get(&codec, "mimeType")?
                .as_string()
                .is_some_and(|s| s.eq_ignore_ascii_case("video/AV1"))
            {
                av1.push(&codec);
            }
        }
        invoke(&transceiver, "setCodecPreferences", &[av1.into()])?;
        let sender = get(&transceiver, "sender")?;
        attach(&workers[i * 2], &sender, &call_hex, None)?;
        attach(
            &workers[i * 2],
            &get(&transceiver, "receiver")?,
            &call_hex,
            Some(&crate::call::hex(members[1 - i])),
        )?;
        let params = invoke(&sender, "getParameters", &[])?;
        let encoding = Array::from(&get(&params, "encodings")?).get(0);
        set(&encoding, "maxBitrate", &(7_000_000u32).into())?;
        set(&encoding, "maxFramerate", &fps.into())?;
        promise(invoke(&sender, "setParameters", &[params])?).await?;
        let peer = pcs[1 - i].clone();
        let ice = Closure::<dyn FnMut(JsValue)>::new(move |event| {
            if let Ok(candidate) = get(&event, "candidate") {
                if !candidate.is_null() {
                    let peer = peer.clone();
                    spawn_local(async move {
                        if let Ok(p) = invoke(&peer, "addIceCandidate", &[candidate]) {
                            let _ = promise(p).await;
                        }
                    });
                }
            }
        });
        set(&pcs[i], "onicecandidate", ice.as_ref())?;
        handlers.push(ice);
        let video = document
            .create_element("video")?
            .dyn_into::<web_sys::HtmlVideoElement>()?;
        video.set_muted(true);
        video.set_autoplay(true);
        video.set_attribute("playsinline", "")?;
        video.set_attribute("style", "width:45%;background:black")?;
        video.set_id(&format!("remote{i}"));
        let holder = document.create_element("div")?;
        holder.set_attribute(
            "style",
            "position:relative;display:inline-block;width:45%;aspect-ratio:16/9",
        )?;
        video.set_attribute("style", "width:100%;height:100%")?;
        holder.append_child(&video)?;
        body.append_child(&holder)?;
        let peer_member = crate::call::hex(members[1 - i]);
        crate::video::video_native_attach(peer_member.clone(), video.clone())?;
        let ontrack = Closure::<dyn FnMut(JsValue)>::new(move |event| {
            let result = (|| -> Result<(), JsValue> {
                let stream = construct("MediaStream", &Array::of1(&get(&event, "track")?))?;

                if web_sys::window().unwrap().location().search()? != "?plain" { return Ok(()); }
                video.remove_attribute("hidden")?;
                set(&video, "srcObject", &stream)?;
                let _ = invoke(&video, "play", &[])?;
                Ok(())
            })();
            if let Err(e) = result {
                web_sys::console::error_1(&e);
            }
        });
        set(&pcs[i], "ontrack", ontrack.as_ref())?;
        handlers.push(ontrack);
    }
    let offer = promise(invoke(&pcs[0], "createOffer", &[])?).await?;
    promise(invoke(&pcs[0], "setLocalDescription", &[offer.clone()])?).await?;
    promise(invoke(&pcs[1], "setRemoteDescription", &[offer])?).await?;
    let answer = promise(invoke(&pcs[1], "createAnswer", &[])?).await?;
    promise(invoke(&pcs[1], "setLocalDescription", &[answer.clone()])?).await?;
    promise(invoke(&pcs[0], "setRemoteDescription", &[answer])?).await?;
    let sources = pcs.clone();
    let busy = Rc::new(Cell::new(false));
    let live = running.clone();
    let stats = Closure::<dyn FnMut()>::new(move || {
        if busy.replace(true) {
            return;
        }
        let busy = busy.clone();
        let sources = sources.clone();
        let live = live.clone();
        spawn_local(async move {
            let values = Array::new();
            for pc in sources {
                if let Ok(report) = invoke(&pc, "getStats", &[]) {
                    if let Ok(report) = promise(report).await {
                        let sink = values.clone();
                        let visit = Closure::<dyn FnMut(JsValue)>::new(move |entry| {
                            let ty = get(&entry, "type")
                                .ok()
                                .and_then(|v| v.as_string())
                                .unwrap_or_default();
                            if (ty == "inbound-rtp" || ty == "outbound-rtp")
                                && get(&entry, "kind")
                                    .ok()
                                    .and_then(|v| v.as_string())
                                    .as_deref()
                                    == Some("video")
                            {
                                sink.push(&entry);
                            }
                        });
                        let _ = invoke(&report, "forEach", &[visit.as_ref().clone()]);
                    }
                }
            }
            if live.get() {
                let document = web_sys::window().unwrap().document().unwrap();
                for i in 0..2 {
                    if let Some(canvas) = document
                        .get_element_by_id(&format!("source{i}"))
                        .and_then(|v| v.dyn_into::<web_sys::HtmlCanvasElement>().ok())
                    {
                        let ctx = canvas
                            .get_context("2d")
                            .unwrap()
                            .unwrap()
                            .dyn_into::<web_sys::CanvasRenderingContext2d>()
                            .unwrap();
                        let color = ctx.get_image_data(10.0, 500.0, 1.0, 1.0).unwrap().data().0;
                        if let Ok(v) = object(
                            serde_json::json!({"type":"source_pixels","source":i,"rgba":color}),
                        ) {
                            values.push(&v);
                        }
                    }
                    if let Some(video) = document
                        .get_element_by_id(&format!("remote{i}"))
                        .and_then(|v| v.dyn_into::<web_sys::HtmlVideoElement>().ok())
                    {
                        if video.video_width() > 0 || video.parent_element().and_then(|p| p.query_selector("canvas:not([hidden])").ok().flatten()).is_some() {
                            let canvas = document
                                .create_element("canvas")
                                .unwrap()
                                .dyn_into::<web_sys::HtmlCanvasElement>()
                                .unwrap();
                            canvas.set_width(48);
                            canvas.set_height(27);
                            let ctx = canvas
                                .get_context("2d")
                                .unwrap()
                                .unwrap()
                                .dyn_into::<web_sys::CanvasRenderingContext2d>()
                                .unwrap();
                            let painted = video
                                .parent_element()
                                .and_then(|p| {
                                    p.query_selector("canvas:not([hidden])").ok().flatten()
                                })
                                .map(JsValue::from)
                                .unwrap_or_else(|| video.clone().into());
                            if invoke(
                                &ctx,
                                "drawImage",
                                &[painted, 0.into(), 0.into(), 48.into(), 27.into()],
                            )
                            .is_ok()
                            {
                                if let Ok(pixels) = ctx.get_image_data(0.0, 0.0, 48.0, 27.0) {
                                    let pixels = pixels.data().0;
                                    let mut stamp = 0u64;
                                    for bit in 0..24 {
                                        if pixels[(bit * 2 + 1) * 4] > 128 {
                                            stamp |= 1 << bit;
                                        }
                                    }
                                    let age = ((js_sys::Date::now() as u64) & 0xffffff)
                                        .wrapping_sub(stamp)
                                        & 0xffffff;
                                    let color = &pixels[(20 * 48 + 10) * 4..(20 * 48 + 10) * 4 + 4];
                                    if let Ok(v) = object(
                                        serde_json::json!({"type":"pixels","remote":i,"age_ms":age,"rgba":color}),
                                    ) {
                                        values.push(&v);
                                    }
                                }
                            }
                        }
                    }
                }
                PRESENTED.with(|p|{for (sender,v) in p.borrow().iter(){if let Ok(entry)=object(serde_json::json!({"type":"presented","sender":sender,"frames":v.frames,"max_gap_ms":v.gap})){values.push(&entry);}}});
                if let Some(el) = web_sys::window()
                    .unwrap()
                    .document()
                    .unwrap()
                    .get_element_by_id("stats")
                {
                    el.set_text_content(
                        js_sys::JSON::stringify(&values)
                            .ok()
                            .and_then(|v| v.as_string())
                            .as_deref(),
                    );
                }
            }
            busy.set(false);
        });
    });
    timers.push(
        web_sys::window()
            .unwrap()
            .set_interval_with_callback_and_timeout_and_arguments_0(
                stats.as_ref().unchecked_ref(),
                1000,
            )?,
    );
    callbacks.push(stats);
    RUN.with(|r| {
        *r.borrow_mut() = Some(Run {
            pcs,
            workers,
            timers,
            tracks: captured,
            handlers,
            callbacks,
            running,
        })
    });
    Ok(())
}

fn fixture_updates() -> [serde_json::Value; 2] {
    let call = [1u8; 32];
    let roster = [2u8; 32];
    let members = [[3u8; 32], [4u8; 32]];
    let contexts = members.map(|sender| sigil_calls::Context {
        call,
        roster,
        sender,
        incarnation: sigil_calls::random_id().unwrap(),
    });
    let (a, ka) = sigil_calls::Sender::generate(contexts[0]).unwrap();
    let (b, kb) = sigil_calls::Sender::generate(contexts[1]).unwrap();
    let tracks = members.map(|id| {
        (
            id,
            sigil_calls::Tracks {
                audio: false,
                camera: true,
                screen: false,
            },
        )
    });
    [(a,kb),(b,ka)].into_iter().enumerate().map(|(i,(sender,remote))|serde_json::json!({"call":call,"lease":([5u8;32]),"own":members[i],"expires":(js_sys::Date::now()/1000.0) as u64+3600,"tracks":tracks,"sender_context":contexts[i],"sender":sender.into_handoff().unwrap(),"receiver_contexts":[remote.context],"receivers":[remote]})).collect::<Vec<_>>().try_into().unwrap_or_else(|_|unreachable!())
}
