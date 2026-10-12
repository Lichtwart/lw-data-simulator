//! Cumulocity media types (`application/vnd.com.nsn.cumulocity.*+json`) and which one a
//! request uses.
//!
//! [`for_request`] derives `Accept` / `Content-Type` from method, path and body, so the generic
//! helpers (`get_json`, `post_json`, …) send the right headers without the caller naming them.
//! The same function renders "copy as curl" (`activity::to_curl`), so the log needs no headers.

/// `application/json` (fallback, realtime).
pub const JSON: &str = "application/json";
pub const MANAGED_OBJECT: &str = "application/vnd.com.nsn.cumulocity.managedobject+json";
pub const MANAGED_OBJECT_COLLECTION: &str =
    "application/vnd.com.nsn.cumulocity.managedobjectcollection+json";
pub const MANAGED_OBJECT_REFERENCE: &str =
    "application/vnd.com.nsn.cumulocity.managedobjectreference+json";
pub const MANAGED_OBJECT_REFERENCE_COLLECTION: &str =
    "application/vnd.com.nsn.cumulocity.managedobjectreferencecollection+json";
pub const MEASUREMENT: &str = "application/vnd.com.nsn.cumulocity.measurement+json";
pub const MEASUREMENT_COLLECTION: &str =
    "application/vnd.com.nsn.cumulocity.measurementcollection+json";
pub const ALARM: &str = "application/vnd.com.nsn.cumulocity.alarm+json";
pub const ALARM_COLLECTION: &str = "application/vnd.com.nsn.cumulocity.alarmcollection+json";
pub const EVENT: &str = "application/vnd.com.nsn.cumulocity.event+json";
pub const EVENT_COLLECTION: &str = "application/vnd.com.nsn.cumulocity.eventcollection+json";
pub const OPERATION: &str = "application/vnd.com.nsn.cumulocity.operation+json";
pub const OPERATION_COLLECTION: &str =
    "application/vnd.com.nsn.cumulocity.operationcollection+json";
pub const CURRENT_USER: &str = "application/vnd.com.nsn.cumulocity.currentuser+json";
/// Binary upload (`POST /inventory/binaries`).
pub const MULTIPART: &str = "multipart/form-data";

/// Headers of one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaTypes {
    pub accept: &'static str,
    /// `None` when the request has no body.
    pub content_type: Option<&'static str>,
}

/// `Accept` / `Content-Type` for a request. `path` may carry a query string; `body` is the
/// JSON body (used to tell a `measurementCollection` from a single measurement).
pub fn for_request(method: &str, path: &str, body: Option<&serde_json::Value>) -> MediaTypes {
    let path = path.split('?').next().unwrap_or(path);
    let segs: Vec<&str> = path.trim_matches('/').split('/').collect();
    let method = method.to_ascii_uppercase();
    let has_body = body.is_some();
    let ct = |t: &'static str| has_body.then_some(t);
    let single_or_collection = |single: &'static str, collection: &'static str| {
        if method == "GET" {
            MediaTypes {
                accept: collection,
                content_type: None,
            }
        } else {
            MediaTypes {
                accept: single,
                content_type: ct(single),
            }
        }
    };

    match segs.as_slice() {
        ["user", "currentUser"] => MediaTypes {
            accept: CURRENT_USER,
            content_type: ct(CURRENT_USER),
        },
        ["inventory", "binaries", ..] => MediaTypes {
            accept: MANAGED_OBJECT,
            content_type: if method == "POST" {
                Some(MULTIPART)
            } else {
                ct(JSON)
            },
        },
        [
            "inventory",
            "managedObjects",
            _,
            "childAdditions" | "childAssets" | "childDevices",
        ] => {
            if method == "GET" {
                MediaTypes {
                    accept: MANAGED_OBJECT_REFERENCE_COLLECTION,
                    content_type: None,
                }
            } else {
                MediaTypes {
                    accept: MANAGED_OBJECT_REFERENCE,
                    content_type: ct(MANAGED_OBJECT_REFERENCE),
                }
            }
        }
        ["inventory", "managedObjects", _, ..] => MediaTypes {
            accept: MANAGED_OBJECT,
            content_type: ct(MANAGED_OBJECT),
        },
        ["inventory", "managedObjects"] => {
            single_or_collection(MANAGED_OBJECT, MANAGED_OBJECT_COLLECTION)
        }
        ["measurement", "measurements"] => {
            let is_collection = body.is_some_and(|b| {
                b.get("measurements")
                    .is_some_and(serde_json::Value::is_array)
            });
            if method == "GET" {
                MediaTypes {
                    accept: MEASUREMENT_COLLECTION,
                    content_type: None,
                }
            } else if is_collection {
                MediaTypes {
                    accept: MEASUREMENT_COLLECTION,
                    content_type: Some(MEASUREMENT_COLLECTION),
                }
            } else {
                MediaTypes {
                    accept: MEASUREMENT,
                    content_type: ct(MEASUREMENT),
                }
            }
        }
        ["measurement", "measurements", _] => MediaTypes {
            accept: MEASUREMENT,
            content_type: ct(MEASUREMENT),
        },
        ["alarm", "alarms"] => single_or_collection(ALARM, ALARM_COLLECTION),
        ["alarm", "alarms", _] => MediaTypes {
            accept: ALARM,
            content_type: ct(ALARM),
        },
        ["event", "events"] => single_or_collection(EVENT, EVENT_COLLECTION),
        ["event", "events", _] => MediaTypes {
            accept: EVENT,
            content_type: ct(EVENT),
        },
        ["devicecontrol", "operations"] => single_or_collection(OPERATION, OPERATION_COLLECTION),
        ["devicecontrol", "operations", _] => MediaTypes {
            accept: OPERATION,
            content_type: ct(OPERATION),
        },
        _ => MediaTypes {
            accept: JSON,
            content_type: ct(JSON),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn measurements_single_vs_collection() {
        let single = json!({ "type": "x" });
        let coll = json!({ "measurements": [] });
        assert_eq!(
            for_request("POST", "/measurement/measurements", Some(&single)),
            MediaTypes {
                accept: MEASUREMENT,
                content_type: Some(MEASUREMENT)
            }
        );
        assert_eq!(
            for_request("POST", "/measurement/measurements", Some(&coll)),
            MediaTypes {
                accept: MEASUREMENT_COLLECTION,
                content_type: Some(MEASUREMENT_COLLECTION)
            }
        );
        assert_eq!(
            for_request("GET", "/measurement/measurements?source=1", None).accept,
            MEASUREMENT_COLLECTION
        );
    }

    #[test]
    fn inventory_and_child_additions() {
        let b = json!({});
        assert_eq!(
            for_request("GET", "/inventory/managedObjects?pageSize=5", None),
            MediaTypes {
                accept: MANAGED_OBJECT_COLLECTION,
                content_type: None
            }
        );
        assert_eq!(
            for_request("PUT", "/inventory/managedObjects/12", Some(&b)).content_type,
            Some(MANAGED_OBJECT)
        );
        assert_eq!(
            for_request(
                "POST",
                "/inventory/managedObjects/12/childAdditions",
                Some(&b)
            ),
            MediaTypes {
                accept: MANAGED_OBJECT_REFERENCE,
                content_type: Some(MANAGED_OBJECT_REFERENCE)
            }
        );
        assert_eq!(
            for_request("GET", "/inventory/managedObjects/12/childAdditions", None).accept,
            MANAGED_OBJECT_REFERENCE_COLLECTION
        );
        assert_eq!(
            for_request("DELETE", "/inventory/managedObjects/12", None).content_type,
            None
        );
        assert_eq!(
            for_request("POST", "/inventory/binaries", None).content_type,
            Some(MULTIPART)
        );
    }

    #[test]
    fn alarms_events_operations_user() {
        let b = json!({});
        assert_eq!(
            for_request("GET", "/alarm/alarms?status=ACTIVE", None).accept,
            ALARM_COLLECTION
        );
        assert_eq!(
            for_request("PUT", "/alarm/alarms/7", Some(&b)).content_type,
            Some(ALARM)
        );
        assert_eq!(
            for_request("POST", "/event/events", Some(&b)).content_type,
            Some(EVENT)
        );
        assert_eq!(
            for_request("GET", "/devicecontrol/operations?deviceId=1", None).accept,
            OPERATION_COLLECTION
        );
        assert_eq!(
            for_request("PUT", "/devicecontrol/operations/9", Some(&b)),
            MediaTypes {
                accept: OPERATION,
                content_type: Some(OPERATION)
            }
        );
        assert_eq!(
            for_request("GET", "/user/currentUser", None).accept,
            CURRENT_USER
        );
        assert_eq!(
            for_request("POST", "/notification/realtime", Some(&b)).content_type,
            Some(JSON)
        );
    }
}
