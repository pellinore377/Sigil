//! Android draws its own emoji.
//!
//! The engine's own renderer cuts a bitmap out of a `CBDT` font, which is
//! how Noto Color Emoji shipped for years. A current phone carries the vector
//! edition instead — three megabytes of `COLRv1` paint graphs with no bitmaps
//! in it at all, and the flags moved out to a second font — so the cut finds
//! nothing and every emoji is reported undrawable. Rather than teach the
//! engine a second font format and a fallback chain, the phone is asked to
//! draw the text the way it draws it everywhere else: `Paint` and `Canvas`
//! through JNI, into a bitmap, out as PNG. That is the platform's own shaper
//! and its own fallback list, so a sequence the phone can show, we can show.
//!
//! The JVM and Activity come from `geo::android`, which the frontend hands
//! them to at startup; without them (a headless test) everything here answers
//! `None` and the engine's own renderer is tried instead.

use anyhow::{Context as _, Result};
use jni::objects::{JByteArray, JObject, JValue};
use jni::JNIEnv;

/// The text size the picture is drawn at, in pixels. Noto's bitmaps are 128
/// square, and the view scales every picture to its cell, so this only has
/// to be large enough that scaling down keeps the edges clean.
const DRAW_PX: i32 = 128;

fn check(env: &mut JNIEnv, what: &str) -> Result<()> {
    if env.exception_check().unwrap_or(false) {
        env.exception_describe().ok();
        env.exception_clear().ok();
        anyhow::bail!("{what} threw");
    }
    Ok(())
}

/// Whether the phone's fonts can draw each of these as one glyph —
/// `Paint.hasGlyph`, which is the very question the picker needs answered.
/// One attach and one `Paint` for the whole list, since the picker asks about
/// two thousand at once.
pub fn can_draw_all(texts: &[&str]) -> Option<Vec<bool>> {
    let vm = crate::geo::android::vm().ok()?;
    let mut guard = vm.attach_current_thread().ok()?;
    let env: &mut JNIEnv = &mut guard;
    let paint = env.new_object("android/graphics/Paint", "()V", &[]).ok()?;
    let mut out = Vec::with_capacity(texts.len());
    for text in texts {
        let js = match env.new_string(text) {
            Ok(s) => s,
            Err(_) => {
                out.push(false);
                continue;
            }
        };
        let r = env.call_method(&paint, "hasGlyph", "(Ljava/lang/String;)Z", &[JValue::Object(&js)]);
        let ok = if env.exception_check().unwrap_or(false) {
            env.exception_clear().ok();
            false
        } else {
            r.ok().and_then(|v| v.z().ok()).unwrap_or(false)
        };
        // Two thousand strings would overflow the local reference table.
        env.delete_local_ref(js).ok();
        out.push(ok);
    }
    Some(out)
}

/// The emoji as a PNG, drawn by the phone; `None` when the phone cannot
/// draw it as one glyph, or when there is no JVM to ask.
pub fn render_png(text: &str) -> Option<Vec<u8>> {
    match draw(text) {
        Ok(png) => Some(png),
        Err(e) => {
            tracing::debug!("emoji: the phone did not draw {text:?}: {e:#}");
            None
        }
    }
}

fn draw(text: &str) -> Result<Vec<u8>> {
    let vm = crate::geo::android::vm()?;
    let mut guard = vm.attach_current_thread().context("attach to the JVM")?;
    let env: &mut JNIEnv = &mut guard;

    // Paint(ANTI_ALIAS_FLAG), at the drawing size.
    let paint = env.new_object("android/graphics/Paint", "(I)V", &[JValue::Int(1)])?;
    check(env, "Paint")?;
    env.call_method(&paint, "setTextSize", "(F)V", &[JValue::Float(DRAW_PX as f32)])?;
    check(env, "setTextSize")?;

    let js = env.new_string(text)?;
    let has = env
        .call_method(&paint, "hasGlyph", "(Ljava/lang/String;)Z", &[JValue::Object(&js)])?
        .z()?;
    check(env, "hasGlyph")?;
    if !has {
        anyhow::bail!("no glyph");
    }

    // The box: the advance across, ascent to descent down, a margin round.
    let advance = env
        .call_method(&paint, "measureText", "(Ljava/lang/String;)F", &[JValue::Object(&js)])?
        .f()?;
    check(env, "measureText")?;
    let metrics = env
        .call_method(&paint, "getFontMetricsInt", "()Landroid/graphics/Paint$FontMetricsInt;", &[])?
        .l()?;
    check(env, "getFontMetricsInt")?;
    let ascent = env.get_field(&metrics, "ascent", "I")?.i()?; // negative
    let descent = env.get_field(&metrics, "descent", "I")?.i()?;
    let pad = DRAW_PX / 8;
    let w = (advance.ceil() as i32).max(1) + 2 * pad;
    let h = (descent - ascent).max(1) + 2 * pad;

    let config = env
        .get_static_field(
            "android/graphics/Bitmap$Config",
            "ARGB_8888",
            "Landroid/graphics/Bitmap$Config;",
        )?
        .l()?;
    let bitmap = env
        .call_static_method(
            "android/graphics/Bitmap",
            "createBitmap",
            "(IILandroid/graphics/Bitmap$Config;)Landroid/graphics/Bitmap;",
            &[JValue::Int(w), JValue::Int(h), JValue::Object(&config)],
        )?
        .l()?;
    check(env, "createBitmap")?;
    let canvas = env.new_object(
        "android/graphics/Canvas",
        "(Landroid/graphics/Bitmap;)V",
        &[JValue::Object(&bitmap)],
    )?;
    check(env, "Canvas")?;
    env.call_method(
        &canvas,
        "drawText",
        "(Ljava/lang/String;FFLandroid/graphics/Paint;)V",
        &[
            JValue::Object(&js),
            JValue::Float(pad as f32),
            JValue::Float((pad - ascent) as f32),
            JValue::Object(&paint),
        ],
    )?;
    check(env, "drawText")?;

    let out = env.new_object("java/io/ByteArrayOutputStream", "()V", &[])?;
    let png = env
        .get_static_field(
            "android/graphics/Bitmap$CompressFormat",
            "PNG",
            "Landroid/graphics/Bitmap$CompressFormat;",
        )?
        .l()?;
    let wrote = env
        .call_method(
            &bitmap,
            "compress",
            "(Landroid/graphics/Bitmap$CompressFormat;ILjava/io/OutputStream;)Z",
            &[JValue::Object(&png), JValue::Int(100), JValue::Object(&out)],
        )?
        .z()?;
    check(env, "compress")?;
    if !wrote {
        anyhow::bail!("compress refused");
    }
    let bytes: JObject = env.call_method(&out, "toByteArray", "()[B", &[])?.l()?;
    let bytes = JByteArray::from(bytes);
    let data = env.convert_byte_array(&bytes)?;
    env.call_method(&bitmap, "recycle", "()V", &[]).ok();
    Ok(data)
}
