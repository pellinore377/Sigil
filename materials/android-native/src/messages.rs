use super::*;
use jni::objects::{JFloatArray, JString};
use jni::sys::{jboolean, jfloatArray, jlong};
use std::collections::HashMap;
thread_local! { static VIEWS: RefCell<HashMap<i64, State>> = RefCell::new(HashMap::new()); }
fn guarded<T: Default>(f: impl FnOnce() -> Option<T>) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .ok()
        .flatten()
        .unwrap_or_default()
}
#[no_mangle]
pub extern "system" fn Java_org_sigil_compose_MaterialNative_create(
    env: JNIEnv,
    _class: JClass,
    surface: JObject,
    width: jint,
    height: jint,
) -> jlong {
    guarded(|| {
        if !(1..=512).contains(&width) || !(1..=512).contains(&height) {
            return None;
        }
        VIEWS.with(|views| {
            let mut views = views.borrow_mut();
            if views.len() >= 18 {
                // A departing timeline row may still own its surface. The caller
                // retries after disposal instead of permanently losing this object.
                return Some(-1);
            }
            // JNI surface is live during this call; the returned window owns a native reference.
            let window = unsafe { NativeWindow::from_surface(env.get_raw(), surface.as_raw()) }?;
            let state = State::new(window, width as u32, height as u32).ok()?;
            static NEXT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);
            let id = NEXT
                .fetch_update(
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                    |v| v.checked_add(1),
                )
                .ok()?;
            views.insert(id, state);
            Some(id)
        })
    })
}
#[no_mangle]
pub extern "system" fn Java_org_sigil_compose_MaterialNative_draw(
    mut env: JNIEnv,
    _class: JClass,
    id: jlong,
    kind: jint,
    sides: jint,
    face: jint,
    font: jint,
    accent: jint,
    backdrop: jint,
    progress: jfloat,
    rotation: JFloatArray,
    label: JString,
    transparent: jboolean,
    style: JFloatArray,
) -> jint {
    guarded(|| {
        VIEWS.with(|views| {
            let mut views = views.borrow_mut();
            let state = views.get_mut(&id)?;
            let object = match kind {
                0 => Object::Die,
                1 => Object::Coin,
                2 => Object::Card,
                _ => return None,
            };
            let die = if object == Object::Die {
                Die::ALL.into_iter().find(|d| d.sides() as i32 == sides)?
            } else {
                Die::D6
            };
            let mut pose = if object == Object::Card {
                sigil_materials::physics::Pose {
                    position: glam::Vec3::ZERO,
                    rotation: glam::Quat::IDENTITY,
                }
            } else {
                sigil_materials::presentation::result_pose(
                    object,
                    die,
                    face.try_into().ok()?,
                    progress,
                )?
            };
            let custom = !rotation.is_null();
            if custom {
                if env.get_array_length(&rotation).ok()? != 4 {
                    return None;
                }
                let mut q = [0.; 4];
                env.get_float_array_region(&rotation, 0, &mut q).ok()?;
                let q = glam::Quat::from_array(q);
                if !q.is_finite() || (q.length() - 1.).abs() > 0.01 {
                    return None;
                }
                pose.rotation = q.normalize();
                pose.position = glam::Vec3::ZERO;
            }
            let label: String = if label.is_null() {
                String::new()
            } else {
                env.get_string(&label).ok()?.into()
            };
            if label.len() > 4096 {
                return None;
            }
            if ![0, 1].contains(&font) {
                return None;
            }
            if state.selection.2 != font || state.label != label {
                state.renderer.set_label(
                    &state.queue,
                    &label,
                    if font == 0 {
                        Lettering::Newsreader
                    } else {
                        Lettering::Sans
                    },
                );
                state.selection.2 = font;
                state.label = label;
            }
            let rgb = |c: i32| {
                [
                    ((c >> 16) & 255) as f32 / 255.,
                    ((c >> 8) & 255) as f32 / 255.,
                    (c & 255) as f32 / 255.,
                ]
            };
            state.scene = Scene {
                object,
                die,
                pose: Some(pose),
                backdrop: Some(rgb(backdrop)),
                transparent: transparent != 0,
                orthographic: object != Object::Card,
                samples: if progress < 1. { 1 } else { 4 },
                numbering: if object==Object::Die {match state.label.as_str() {"tens"=>1,"units"=>2,_=>0}} else {0},
                mode: Mode::Conversation,
                conversation: Some(rgb(accent)),
                zoom: if object == Object::Card { 4.0 } else { 2.7 },
                ..Default::default()
            };
            if !style.is_null() {
                let count=env.get_array_length(&style).ok()?;
                if count!=12 && count!=13 {return None;}
                let mut p=[0.;13];
                env.get_float_array_region(&style,0,&mut p[..count as usize]).ok()?;
                if !p.iter().all(|v|v.is_finite()) || !p[..3].iter().all(|v|(0. ..=16777215.).contains(v) && v.fract()==0.) || !(0. ..=2.).contains(&p[11]) {return None;}
                let a=&mut state.scene.appearances[object as usize];
                a.color=rgb(p[0] as i32);a.second=rgb(p[1] as i32);a.ink=rgb(p[2] as i32);
                a.roughness=p[3];a.transmission=p[4];a.absorption=p[5];a.ior=p[6];
                a.roundness=p[7];a.engraving=p[8];a.inclusions=p[9];
                state.scene.border=p[10].clamp(0.,2.) as u32;
                state.scene.texture_style=p[12].clamp(0.,3.) as u32;
                state.scene.mode=match p[11] as u32 {0=>Mode::Personalized,1=>Mode::Global,_=>Mode::Conversation};
                state.scene.global=rgb(accent);
            }
            let frame = match state.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(f)
                | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    return Some(2)
                }
                _ => return None,
            };
            state.renderer.render_to(
                &state.device,
                &state.queue,
                &state.scene,
                &frame.texture.create_view(&Default::default()),
            );
            state.queue.present(frame);
            Some(1)
        })
    })
}
#[no_mangle]
pub extern "system" fn Java_org_sigil_compose_MaterialNative_destroy(
    _env: JNIEnv,
    _class: JClass,
    id: jlong,
) {
    guarded(|| {
        VIEWS.with(|views| {
            if let Some(state) = views.borrow_mut().remove(&id) {
                let _ = wait(&state.device);
                drop(state);
            }
            Some(())
        })
    });
}

#[no_mangle]
pub extern "system" fn Java_org_sigil_compose_MaterialNative_record(
    env: JNIEnv,
    _class: JClass,
    data: JFloatArray,
) -> jfloatArray {
    guarded(|| {
        let len = env.get_array_length(&data).ok()? as usize;
        if !(8..=300).contains(&len) {
            return None;
        }
        let mut a = vec![0.; len];
        env.get_float_array_region(&data, 0, &mut a).ok()?;
        let output=sigil_materials::timeline::record_packed(&a)?;
        let result = env.new_float_array(output.len() as i32).ok()?;
        env.set_float_array_region(&result, 0, &output).ok()?;
        Some(result.into_raw())
    })
}

#[no_mangle]
pub extern "system" fn Java_org_sigil_compose_MaterialNative_horizontalExtent(
    env: JNIEnv, _class: JClass, sides: jint, face: jint, rotation: JFloatArray, outgoing: jboolean,
) -> jfloat {
    guarded(|| {
        if sides == 0 {return Some(0.972);}
        let die=Die::ALL.into_iter().find(|d|d.sides() as i32==sides)?;
        let mut q=sigil_materials::presentation::result_pose(Object::Die,die,face.try_into().ok()?,1.)?.rotation;
        if !rotation.is_null() {
            if env.get_array_length(&rotation).ok()?!=4 {return None;}
            let mut values=[0.;4];env.get_float_array_region(&rotation,0,&mut values).ok()?;
            q=glam::Quat::from_array(values);
            if !q.is_finite() || (q.length()-1.).abs()>0.01 {return None;}
        }
        Some(die.hull().vertices.iter().map(|v|(q * *v).x*if outgoing!=0 {1.}else{-1.}).fold(0.,f32::max))
    })
}
