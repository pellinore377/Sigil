use wasm_bindgen::{prelude::*, JsCast};

#[wasm_bindgen]
pub struct LocationWatch {
    geolocation: web_sys::Geolocation,
    id: i32,
    _success: Closure<dyn FnMut(web_sys::Position)>,
    _failure: Closure<dyn FnMut(web_sys::PositionError)>,
}

#[wasm_bindgen]
impl LocationWatch {
    #[wasm_bindgen(constructor)]
    pub fn new(changed: js_sys::Function, failed: js_sys::Function) -> Result<LocationWatch, JsValue> {
        let window = web_sys::window().ok_or_else(|| crate::fail("Location requires a browser window"))?;
        let geolocation = window.navigator().geolocation()?;
        let success = Closure::wrap(Box::new(move |position: web_sys::Position| {
            let c = position.coords();
            let lat = c.latitude();
            let lon = c.longitude();
            let accuracy = c.accuracy();
            let timestamp = position.timestamp();
            let now = js_sys::Date::now();
            if !lat.is_finite() || !lon.is_finite() || !accuracy.is_finite()
                || !timestamp.is_finite() || !(-90.0..=90.0).contains(&lat)
                || !(-180.0..=180.0).contains(&lon) || accuracy < 0.0
                || timestamp > now || now - timestamp > 60_000.0 { return; }
            let point = serde_json::json!({"coordinates":{"latitude_e6":(lat*1_000_000.0).round() as i32,
                "longitude_e6":(lon*1_000_000.0).round() as i32},"accuracy_cm":(accuracy*100.0).min(u32::MAX as f64).round() as u32,
                "sampled_at":(timestamp/1000.0) as u64});
            let _ = changed.call1(&JsValue::NULL, &point.to_string().into());
        }) as Box<dyn FnMut(web_sys::Position)>);
        let failure = Closure::wrap(Box::new(move |error: web_sys::PositionError| {
            let text = match error.code() { 1 => "Location access was denied.", 3 => "Location timed out. Retry when ready.", _ => "Your location is unavailable." };
            let _ = failed.call1(&JsValue::NULL, &text.into());
        }) as Box<dyn FnMut(web_sys::PositionError)>);
        let options = web_sys::PositionOptions::new();
        options.set_enable_high_accuracy(true);
        options.set_maximum_age(0);
        options.set_timeout(20_000);
        let id = geolocation.watch_position_with_error_callback_and_options(success.as_ref().unchecked_ref(), Some(failure.as_ref().unchecked_ref()), &options)?;
        Ok(Self {geolocation,id,_success:success,_failure:failure})
    }
}
impl Drop for LocationWatch {
    fn drop(&mut self) {self.geolocation.clear_watch(self.id);}
}
