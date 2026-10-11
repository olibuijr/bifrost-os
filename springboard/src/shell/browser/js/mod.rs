//! JavaScript for the browser: one Boa `Context` per tab, bound to a shared
//! `Dom`. The Web API surface is a JavaScript prelude (`events`, `url`,
//! `storage`, `element`, `document`, `timers`, `net`, `window`) running over a
//! small set of native `__h_*` bridge functions that touch the `Dom`, the
//! timer queue and the request queue. `page::JsPage` is the entry point.

mod document;
mod element;
mod events;
pub mod limits;
mod net;
mod page;
pub mod selector;
mod storage;
mod timers;
mod url;
mod window;

pub use net::{NetRequest, NetResponse};
pub use page::{EventInit, JsPage};

use super::dom::{Dom, NodeId};
use boa_engine::{Context, JsResult, JsString, JsValue, NativeFunction};
use std::cell::RefCell;
use std::rc::Rc;

/// Signature of every native bridge function.
pub(crate) type NativeFn = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;

/// State the native functions reach through the context's host data.
pub(crate) struct Host {
    pub dom: Rc<RefCell<Dom>>,
    pub base_url: String,
    pub log: RefCell<Vec<String>>,
    pub timers: RefCell<timers::Timers>,
    pub net: RefCell<net::Net>,
}

/// The host of the running context.
pub(crate) fn host(ctx: &Context) -> Rc<Host> {
    ctx.get_data::<Rc<Host>>().cloned().expect("JsPage host data is installed")
}

pub(crate) fn arg_str(args: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<String> {
    match args.get(i) {
        Some(v) => Ok(v.to_string(ctx)?.to_std_string_escaped()),
        None => Ok(String::new()),
    }
}

pub(crate) fn arg_num(args: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<f64> {
    match args.get(i) {
        Some(v) => v.to_number(ctx),
        None => Ok(f64::NAN),
    }
}

pub(crate) fn arg_bool(args: &[JsValue], i: usize) -> bool {
    args.get(i).is_some_and(JsValue::to_boolean)
}

/// A node id argument; negative or NaN values map to `None`.
pub(crate) fn arg_opt_node(args: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<Option<NodeId>> {
    let n = arg_num(args, i, ctx)?;
    Ok(if n.is_finite() && n >= 0.0 { Some(NodeId(n as u32)) } else { None })
}

pub(crate) fn arg_node(args: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<NodeId> {
    match arg_opt_node(args, i, ctx)? {
        Some(n) => Ok(n),
        None => Err(boa_engine::JsNativeError::typ().with_message("invalid node").into()),
    }
}

pub(crate) fn js_string(s: &str) -> JsValue {
    JsValue::from(JsString::from(s))
}

/// A node or -1 for none.
pub(crate) fn node_value(n: Option<NodeId>) -> JsValue {
    JsValue::from(n.map_or(-1.0, |n| f64::from(n.0)))
}

/// A list of nodes as a comma separated string (the prelude splits it).
pub(crate) fn node_list(ids: impl Iterator<Item = NodeId>) -> JsValue {
    let v: Vec<String> = ids.map(|n| n.0.to_string()).collect();
    js_string(&v.join(","))
}

/// Defines each native as a global `__h_<name>`; the prelude gathers them.
pub(crate) fn register(ctx: &mut Context, table: &[(&str, NativeFn)]) {
    for (name, f) in table {
        let key = JsString::from(format!("__h_{name}").as_str());
        ctx.register_global_callable(key, 0, NativeFunction::from_fn_ptr(*f))
            .expect("register native");
    }
}

/// The whole prelude: the native table first, then every module's JavaScript
/// inside one function scope so they share their helpers.
pub(crate) fn prelude() -> String {
    let parts = [
        events::JS,
        url::JS,
        storage::JS,
        element::JS,
        document::JS,
        timers::JS,
        net::JS,
        window::JS,
    ];
    let mut s = String::from(
        "(function(){'use strict';const H={};\
         for(const k of Object.getOwnPropertyNames(globalThis)){\
         if(k.startsWith('__h_')){H[k.slice(4)]=globalThis[k];delete globalThis[k];}}\n",
    );
    for p in parts {
        s.push_str(p);
        s.push('\n');
    }
    s.push_str("})();");
    s
}

/// Registers every native bridge function.
pub(crate) fn register_all(ctx: &mut Context) {
    element::register(ctx);
    document::register(ctx);
    window::register(ctx);
    timers::register(ctx);
    net::register(ctx);
    url::register(ctx);
}
