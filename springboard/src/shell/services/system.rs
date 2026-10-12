//! The `system` service: `ping` echoes its parameters, `services` lists the registered service names. Apps and tests
//! use it to check that the service channel works.
use super::{Shell, SystemService};
use bifrost_core::json::Json;

pub struct System;

impl SystemService for System {
    fn name(&self) -> &'static str {
        "system"
    }

    fn call(&mut self, _shell: &mut Shell, _app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            "ping" => Ok(params.clone()),
            // the shell's own list is lent out during a call, so read the registry itself
            "services" => Ok(Json::Arr(super::names(&super::all()).into_iter().map(|n| Json::Str(n.into())).collect())),
            _ => Err(format!("system has no method {method}")),
        }
    }
}
