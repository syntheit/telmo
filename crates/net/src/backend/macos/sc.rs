//! SystemConfiguration dynamic store: reads values as JSON and reports changes.

use objc2_core_foundation::{
    CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFRunLoop, CFString, CFType,
    kCFRunLoopDefaultMode,
};
use objc2_system_configuration::{SCDynamicStore, SCDynamicStoreContext};
use serde_json::{Map, Value};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::mpsc::Sender;

/// Keys and patterns whose changes should trigger a new snapshot.
const WATCHED: &[&str] = &[
    "State:/Network/Interface/.*/(IPv4|Link|AirPort)",
    "State:/Network/Global/(IPv4|DNS)",
    "State:/Network/Service/.*/(IPv4|DNS)",
    "Setup:/Network/Service/.*",
    "Setup:/Network/Global/IPv4",
];

pub struct Store {
    store: CFRetained<SCDynamicStore>,
}

impl Store {
    /// A store that only reads values.
    pub fn reader() -> Result<Store, String> {
        let store = unsafe {
            SCDynamicStore::new(
                None,
                &CFString::from_str("telmo-net"),
                None,
                std::ptr::null_mut(),
            )
        };
        store.map(|store| Store { store }).ok_or_else(open_error)
    }

    /// A store that sends `()` on `changed` when a watched key changes. The
    /// notifications arrive while the current thread runs its run loop.
    pub fn watcher(changed: Box<Sender<()>>) -> Result<Store, String> {
        let mut context = SCDynamicStoreContext {
            version: 0,
            info: Box::into_raw(changed).cast::<c_void>(),
            retain: None,
            release: None,
            copyDescription: None,
        };
        let store = unsafe {
            SCDynamicStore::new(
                None,
                &CFString::from_str("telmo-net"),
                Some(notify),
                &mut context,
            )
        };
        let store = Store {
            store: store.ok_or_else(open_error)?,
        };
        store.watch()?;
        Ok(store)
    }

    fn watch(&self) -> Result<(), String> {
        let patterns: Vec<_> = WATCHED.iter().map(|p| CFString::from_str(p)).collect();
        let refs: Vec<&CFString> = patterns.iter().map(|p| &**p).collect();
        let patterns = CFArray::from_objects(&refs);
        let source = SCDynamicStore::new_run_loop_source(None, &self.store, 0);
        let (Some(source), Some(run_loop)) = (source, CFRunLoop::current()) else {
            return Err("Couldn't watch the network configuration for changes.".to_string());
        };
        let registered = unsafe {
            self.store
                .set_notification_keys(None, Some(patterns.as_opaque()))
        };
        if !registered {
            return Err("Couldn't watch the network configuration for changes.".to_string());
        }
        run_loop.add_source(Some(&source), unsafe { kCFRunLoopDefaultMode });
        Ok(())
    }

    pub fn value(&self, key: &str) -> Option<Value> {
        let value = SCDynamicStore::value(Some(&self.store), &CFString::from_str(key))?;
        Some(to_json(&value))
    }

    /// Raw bytes of a data value (the dynamic store keeps some records archived).
    pub fn data(&self, key: &str, field: &str) -> Option<Vec<u8>> {
        let value = SCDynamicStore::value(Some(&self.store), &CFString::from_str(key))?;
        let dict = value.downcast_ref::<CFDictionary>()?;
        let dict =
            unsafe { &*(dict as *const CFDictionary as *const CFDictionary<CFString, CFType>) };
        let field = dict.get(&CFString::from_str(field))?;
        let data = field.downcast_ref::<objc2_core_foundation::CFData>()?;
        Some(data.to_vec())
    }
}

fn open_error() -> String {
    "Couldn't open the network configuration. Is configd running?".to_string()
}

unsafe extern "C-unwind" fn notify(
    _: NonNull<SCDynamicStore>,
    _: NonNull<CFArray>,
    info: *mut c_void,
) {
    // `info` is the leaked Sender from `Store::watcher`; it lives as long as the thread.
    let changed = unsafe { &*info.cast::<Sender<()>>() };
    let _ = changed.send(());
}

/// View an untyped CFArray as holding `T`. The caller knows what's inside.
pub fn typed<T>(array: &CFArray) -> &CFArray<T> {
    unsafe { &*(array as *const CFArray as *const CFArray<T>) }
}

pub fn to_json(value: &CFType) -> Value {
    if let Some(s) = value.downcast_ref::<CFString>() {
        return Value::String(s.to_string());
    }
    if let Some(b) = value.downcast_ref::<CFBoolean>() {
        return Value::Bool(b.as_bool());
    }
    if let Some(n) = value.downcast_ref::<CFNumber>() {
        return number(n);
    }
    if let Some(a) = value.downcast_ref::<CFArray>() {
        return Value::Array(typed::<CFType>(a).iter().map(|v| to_json(&v)).collect());
    }
    if let Some(d) = value.downcast_ref::<CFDictionary>() {
        let d = unsafe { &*(d as *const CFDictionary as *const CFDictionary<CFString, CFType>) };
        let (keys, values) = d.to_vecs();
        let map: Map<String, Value> = keys
            .iter()
            .zip(values.iter())
            .map(|(k, v)| (k.to_string(), to_json(v)))
            .collect();
        return Value::Object(map);
    }
    Value::Null
}

fn number(n: &CFNumber) -> Value {
    if let Some(i) = n.as_i64() {
        return Value::from(i);
    }
    n.as_f64().map(Value::from).unwrap_or(Value::Null)
}

/// The strings of a JSON array value.
pub fn strings(value: Option<&Value>) -> Vec<String> {
    let items = value.and_then(Value::as_array);
    items
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}
