//! End-to-end tests of `C8yClient` and the realtime subscription against [`super::fake`].

use std::time::Duration;

use serde_json::{Value, json};

use super::api::{AlarmStatus, ManagedObjectQuery};
use super::fake::{FakeC8y, PASSWORD};
use super::media;
use super::realtime::{RealtimeConfig, RealtimeMsg, RealtimeSubscription, subscribe_device_with};
use crate::error::AppError;
use crate::model::OperationStatus;

#[tokio::test]
async fn current_user_sends_basic_auth_and_logs_without_password() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let user = client.current_user().await.unwrap();
    assert_eq!(user.user_id, "demo");
    assert_eq!(user.username, "demo");
    assert_eq!(user.display_name.as_deref(), Some("Demo User"));
    assert_eq!(user.tenant.key, "fake");

    let req = fake.requests().pop().unwrap();
    assert_eq!(req.auth_user.as_deref(), Some("t100/demo"));
    assert_eq!(req.use_xbasic.as_deref(), Some("true"));
    assert_eq!(req.accept.as_deref(), Some(media::CURRENT_USER));

    let entries = fake.sink.entries.lock().unwrap().clone();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(
        (e.method.as_str(), e.path.as_str()),
        ("GET", "/user/currentUser")
    );
    assert_eq!(e.status, Some(200));
    assert_eq!(e.user, "t100/demo");
    assert_eq!(e.base_url, fake.base_url);
    let dump = serde_json::to_string(e).unwrap();
    assert!(!dump.contains(PASSWORD));
}

#[tokio::test]
async fn wrong_password_is_auth_failed_without_expiry() {
    let fake = FakeC8y::start().await;
    let client = fake.client_with("demo", "wrong");
    match client.current_user().await {
        Err(AppError::AuthFailed(Some(reason))) => {
            assert!(reason.contains("Invalid credentials"), "{reason}");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(fake.sink.expired.lock().unwrap().is_empty());
    assert_eq!(fake.sink.entries.lock().unwrap()[0].status, Some(401));
}

#[tokio::test]
async fn unauthorized_call_reports_auth_expired() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    fake.state().password = "rotated".into();
    let err = client.get_managed_object("1").await.unwrap_err();
    assert!(matches!(err, AppError::NotAuthenticated));
    assert_eq!(*fake.sink.expired.lock().unwrap(), vec!["fake".to_owned()]);
}

#[tokio::test]
async fn http_and_network_errors_are_mapped() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    match client.get_managed_object("404404").await {
        Err(AppError::Http { status, message }) => {
            assert_eq!(status, 404);
            assert!(message.contains("Not Found"));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(
        client
            .find_managed_object("404404")
            .await
            .unwrap()
            .is_none()
    );

    fake.fail_next("POST", "/event/events", 500);
    let err = client
        .create_event(&json!({ "type": "x", "text": "t", "source": { "id": "1" } }))
        .await
        .unwrap_err();
    assert!(super::is_http_status(&err, 500));

    // Port 1 never accepts connections (no freed port that a parallel test could reuse).
    let tenant = crate::model::Tenant {
        base_url: "http://127.0.0.1:1".into(),
        ..fake.tenant()
    };
    let offline = super::C8yClient::new(
        tenant,
        "demo".into(),
        "x".into(),
        std::sync::Arc::new(super::ActivityLog::new()),
        std::sync::Arc::new(super::NoopSink),
    )
    .unwrap();
    assert!(matches!(
        offline.get_managed_object("1").await,
        Err(AppError::Network(_))
    ));
}

#[tokio::test]
async fn inventory_crud_paging_and_query() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    for i in 0..5 {
        let types = if i % 2 == 0 {
            json!(["demo"])
        } else {
            json!(["real"])
        };
        client
            .create_managed_object(
                &json!({ "name": format!("dev {i}"), "c8y_IsDevice": {}, "c8y_DeviceTypes": types }),
            )
            .await
            .unwrap();
    }
    let all = client
        .query_managed_objects(&ManagedObjectQuery {
            fragment_type: Some("c8y_IsDevice".into()),
            page_size: Some(2),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(all.len(), 5);
    let gets = fake
        .requests()
        .iter()
        .filter(|r| r.method == "GET" && r.path == "/inventory/managedObjects")
        .count();
    assert_eq!(gets, 3, "pages of 2, 2, 1");
    let first = fake
        .requests()
        .into_iter()
        .find(|r| r.method == "GET" && r.path == "/inventory/managedObjects")
        .unwrap();
    assert!(first.query.contains("withTotalPages=true"));

    // `query` cannot be mixed with the plain filters.
    let mixed = client
        .query_managed_objects(&ManagedObjectQuery {
            query: Some("$filter=(has(c8y_IsDevice))".into()),
            object_type: Some("x".into()),
            ..Default::default()
        })
        .await;
    assert!(matches!(mixed, Err(AppError::InvalidInput(_))));

    let demos = client
        .query_managed_objects(&ManagedObjectQuery {
            query: Some("$filter=(has(c8y_IsDevice) and c8y_DeviceTypes eq 'demo')".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(demos.len(), 3);
    let req = fake.requests().pop().unwrap();
    assert_eq!(
        req.accept.as_deref(),
        Some(media::MANAGED_OBJECT_COLLECTION)
    );
    assert!(req.query.contains("pageSize=2000"));

    let id = super::id_of(&demos[0]).unwrap();
    let updated = client
        .update_managed_object(&id, &json!({ "lw_demo": { "schemaVersion": 1 } }))
        .await
        .unwrap();
    assert_eq!(updated["lw_demo"]["schemaVersion"], 1);
    assert_eq!(updated["name"], "dev 0");
    let req = fake.requests().pop().unwrap();
    assert_eq!(req.content_type.as_deref(), Some(media::MANAGED_OBJECT));

    client.delete_managed_object(&id).await.unwrap();
    assert!(client.find_managed_object(&id).await.unwrap().is_none());
}

#[tokio::test]
async fn child_additions_and_binaries() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let device = fake.insert_managed_object(json!({ "name": "d", "c8y_IsDevice": {} }));

    let png = vec![0x89, b'P', b'N', b'G', 0, 1, 2, 255];
    let bin = client
        .upload_binary("plan.png", "image/png", png.clone())
        .await
        .unwrap();
    let bin_id = super::id_of(&bin).unwrap();
    assert_eq!(bin["name"], "plan.png");
    assert_eq!(client.download_binary(&bin_id).await.unwrap(), png);
    let entry = fake.sink.entries.lock().unwrap().last().cloned().unwrap();
    assert_eq!(entry.response_body.as_deref(), Some("[8 bytes]"));

    let fp = client
        .create_managed_object(&json!({ "type": "lw_floorplan", "name": "EG",
                                         "lw_floorplan": { "imageBinaryId": bin_id } }))
        .await
        .unwrap();
    let fp_id = super::id_of(&fp).unwrap();
    client.add_child_addition(&device, &fp_id).await.unwrap();
    let req = fake.requests().pop().unwrap();
    assert_eq!(
        req.content_type.as_deref(),
        Some(media::MANAGED_OBJECT_REFERENCE)
    );
    assert_eq!(req.json(), json!({ "managedObject": { "id": fp_id } }));

    let children = client.list_child_additions(&device).await.unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["type"], "lw_floorplan");

    client.remove_child_addition(&device, &fp_id).await.unwrap();
    assert!(
        client
            .list_child_additions(&device)
            .await
            .unwrap()
            .is_empty()
    );
    client.delete_binary(&bin_id).await.unwrap();
    assert!(client.download_binary(&bin_id).await.is_err());
}

#[tokio::test]
async fn measurements_alarms_events_carry_time_and_media_types() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let src = json!({ "id": "7" });

    client
        .create_measurement(&json!({ "type": "c8y_SignalStrength", "source": src,
                                      "c8y_SignalStrength": { "rssi": { "value": -70, "unit": "dBm" } } }))
        .await
        .unwrap();
    let req = fake.requests().pop().unwrap();
    assert_eq!(req.content_type.as_deref(), Some(media::MEASUREMENT));
    assert!(req.json()["time"].is_string());

    let batch: Vec<Value> = (1..=3)
        .map(|i| json!({ "type": "daliControlGear", "source": src,
                         format!("LW Dali Control Gear 1-0{i}"): { "Brightness": { "value": 50, "unit": "%" } } }))
        .collect();
    client.create_measurements(&batch).await.unwrap();
    let req = fake.requests().pop().unwrap();
    assert_eq!(
        req.content_type.as_deref(),
        Some(media::MEASUREMENT_COLLECTION)
    );
    assert_eq!(req.accept.as_deref(), Some(media::MEASUREMENT_COLLECTION));
    assert_eq!(fake.state().measurements.len(), 4);

    let alarm = json!({ "type": "lw_demo_19_co2Leakage", "text": "CO2", "severity": "MAJOR",
                        "source": src });
    let a1 = client.create_alarm(&alarm).await.unwrap();
    let a2 = client.create_alarm(&alarm).await.unwrap();
    assert_eq!(a1["id"], a2["id"], "open alarms are de-duplicated");
    assert_eq!(a2["count"], 2);
    client
        .create_alarm(&json!({ "type": "other", "text": "o", "severity": "MINOR", "source": src }))
        .await
        .unwrap();
    let a1_id = super::id_of(&a1).unwrap();
    client
        .update_alarm_status(&a1_id, AlarmStatus::Acknowledged)
        .await
        .unwrap();
    assert_eq!(client.open_alarms("7", None).await.unwrap().len(), 2);
    assert_eq!(
        client
            .open_alarms("7", Some("lw_demo_19_co2Leakage"))
            .await
            .unwrap()
            .len(),
        1
    );
    let req = fake.requests().pop().unwrap();
    assert!(req.query.contains("status=ACTIVE%2CACKNOWLEDGED"));
    assert_eq!(req.accept.as_deref(), Some(media::ALARM_COLLECTION));
    // Bulk update per open status (ACTIVE and ACKNOWLEDGED), no PUT per alarm.
    let n_before = fake.requests().len();
    assert_eq!(client.clear_open_alarms("7").await.unwrap(), 2);
    assert!(client.open_alarms("7", None).await.unwrap().is_empty());
    let puts: Vec<_> = fake.requests()[n_before..]
        .iter()
        .filter(|r| r.method == "PUT")
        .cloned()
        .collect();
    assert_eq!(puts.len(), 2, "{puts:?}");
    assert!(puts.iter().all(|r| r.path == "/alarm/alarms"
        && r.query.contains("source=7")
        && r.json() == json!({ "status": "CLEARED" })));
    assert!(puts[0].query.contains("status=ACTIVE"));
    assert!(puts[1].query.contains("status=ACKNOWLEDGED"));
    assert_eq!(
        client.clear_open_alarms("7").await.unwrap(),
        0,
        "nothing open"
    );

    // A failing bulk update falls back to one PUT per alarm.
    let open = |t: &str| json!({ "type": t, "text": t, "severity": "MINOR", "source": src });
    client.create_alarm(&open("b1")).await.unwrap();
    client.create_alarm(&open("b2")).await.unwrap();
    fake.fail_next("PUT", "/alarm/alarms", 500);
    assert_eq!(client.clear_open_alarms("7").await.unwrap(), 2);
    assert!(client.open_alarms("7", None).await.unwrap().is_empty());

    // 202 Accepted: every listed alarm is cleared one by one right away (no polling in between,
    // it would hold up the ordered outbound lane); a failing PUT there does not stop the rest
    // and is returned.
    fake.state().bulk_alarm_update_accepted = true;
    client.create_alarm(&open("c1")).await.unwrap();
    client.create_alarm(&open("c2")).await.unwrap();
    fake.fail_next("PUT", "/alarm/alarms/", 500);
    let n_before = fake.requests().len();
    let err = client.clear_open_alarms("7").await.unwrap_err();
    assert!(super::is_http_status(&err, 500));
    let reqs = fake.requests()[n_before..].to_vec();
    let last_bulk = reqs
        .iter()
        .rposition(|r| r.method == "PUT" && r.path == "/alarm/alarms")
        .unwrap();
    assert!(
        reqs[last_bulk + 1..]
            .iter()
            .all(|r| r.method == "PUT" && r.path.starts_with("/alarm/alarms/")),
        "no re-check before the per-alarm PUTs: {reqs:?}"
    );
    assert_eq!(client.open_alarms("7", None).await.unwrap().len(), 1);
    assert_eq!(client.clear_open_alarms("7").await.unwrap(), 1);
    assert!(client.open_alarms("7", None).await.unwrap().is_empty());

    client
        .create_event(&json!({ "type": "lw_alarm", "text": "Alarm cleared: x", "source": src }))
        .await
        .unwrap();
    let req = fake.requests().pop().unwrap();
    assert_eq!(req.content_type.as_deref(), Some(media::EVENT));
    assert!(req.json()["time"].is_string());
    assert_eq!(
        client
            .query_events("7", Some("lw_alarm"), 5)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn operations_query_and_status() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let op = fake.create_operation("7", json!({ "lw_fetchCore": {} }));
    fake.create_operation("8", json!({ "lw_fetchCore": {} }));
    let op_id = super::id_of(&op).unwrap();

    let pending = client.pending_operations("7").await.unwrap();
    assert_eq!(pending.len(), 1);
    let req = fake.requests().pop().unwrap();
    assert!(req.query.contains("deviceId=7") && req.query.contains("status=PENDING"));
    assert_eq!(req.accept.as_deref(), Some(media::OPERATION_COLLECTION));

    client
        .set_operation_status(&op_id, OperationStatus::Executing, Some("ignored"))
        .await
        .unwrap();
    let req = fake.requests().pop().unwrap();
    assert_eq!(req.json(), json!({ "status": "EXECUTING" }));
    assert_eq!(req.content_type.as_deref(), Some(media::OPERATION));

    client
        .set_operation_status(&op_id, OperationStatus::Failed, Some("Timed out"))
        .await
        .unwrap();
    let req = fake.requests().pop().unwrap();
    assert_eq!(
        req.json(),
        json!({ "status": "FAILED", "failureReason": "Timed out" })
    );
    assert!(client.pending_operations("7").await.unwrap().is_empty());
    assert_eq!(
        client
            .query_operations("7", Some(OperationStatus::Failed))
            .await
            .unwrap()
            .len(),
        1
    );
}

// ───────────────────────────── realtime ─────────────────────────────

fn fast() -> RealtimeConfig {
    RealtimeConfig {
        poll_timeout: Duration::from_millis(300),
        poll_grace: Duration::from_millis(700),
        backoff_min: Duration::from_millis(50),
        backoff_max: Duration::from_millis(200),
    }
}

/// Next message that is not about alarms (the operation tests ignore them).
async fn next(sub: &mut RealtimeSubscription) -> RealtimeMsg {
    loop {
        match next_any(sub).await {
            RealtimeMsg::Alarm(_) | RealtimeMsg::OpenAlarms { .. } => {}
            msg => return msg,
        }
    }
}

async fn next_any(sub: &mut RealtimeSubscription) -> RealtimeMsg {
    tokio::time::timeout(Duration::from_secs(5), sub.recv())
        .await
        .expect("realtime message within 5 s")
        .expect("subscription alive")
}

fn op_id(msg: &RealtimeMsg) -> String {
    match msg {
        RealtimeMsg::Operation(op) | RealtimeMsg::PendingOperation(op) => super::id_of(op).unwrap(),
        other => panic!("expected operation, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realtime_feeds_pending_then_live_operations() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let offline_op = fake.create_operation("7", json!({ "c8y_Restart": {} }));

    let mut sub = subscribe_device_with(&client, "7", fast()).unwrap();
    assert_eq!(next(&mut sub).await, RealtimeMsg::Connected);
    let first = next(&mut sub).await;
    assert!(
        matches!(first, RealtimeMsg::PendingOperation(_)),
        "the PENDING list is marked"
    );
    assert_eq!(op_id(&first), super::id_of(&offline_op).unwrap());

    // Handshake + subscribe went out with the right channel.
    let subscribe = fake
        .requests()
        .into_iter()
        .find(|r| r.path == "/notification/realtime" && r.json()[0]["channel"] == "/meta/subscribe")
        .unwrap();
    assert_eq!(subscribe.json()[0]["subscription"], "/operations/7");
    assert_eq!(subscribe.auth_user.as_deref(), Some("t100/demo"));

    // A live operation arrives via long-poll; other devices are not delivered.
    fake.create_operation("8", json!({ "lw_fetchCore": {} }));
    let live = fake.create_operation(
        "7",
        json!({ "lw_switchGroupMode": { "id": 1, "mode": "on" } }),
    );
    let msg = next(&mut sub).await;
    assert_eq!(op_id(&msg), super::id_of(&live).unwrap());
    if let RealtimeMsg::Operation(op) = &msg {
        assert_eq!(op["lw_switchGroupMode"]["mode"], "on");
        assert_eq!(op["status"], "PENDING");
    }

    // Status updates come back as UPDATE notifications.
    client
        .set_operation_status(
            &super::id_of(&live).unwrap(),
            OperationStatus::Failed,
            Some("x"),
        )
        .await
        .unwrap();
    match next(&mut sub).await {
        RealtimeMsg::Operation(op) => assert_eq!(op["status"], "FAILED"),
        other => panic!("unexpected {other:?}"),
    }

    // Quiet long-polls (every 300 ms here) are not logged; handshake / subscribe and the
    // two connects that carried data are.
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let logged: Vec<String> = fake
        .sink
        .entries
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.path == "/notification/realtime")
        .filter_map(|e| e.request_body.clone())
        .collect();
    assert!(logged.iter().any(|b| b.contains("/meta/handshake")));
    assert!(logged.iter().any(|b| b.contains("/meta/subscribe")));
    let connects = logged
        .iter()
        .filter(|b| b.contains("/meta/connect"))
        .count();
    assert!(connects <= 2, "{connects} connect entries");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realtime_rehandshakes_on_unknown_client_and_refetches_pending() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let mut sub = subscribe_device_with(&client, "7", fast()).unwrap();
    assert_eq!(next(&mut sub).await, RealtimeMsg::Connected);

    // Server "restart": an op created meanwhile is only in the PENDING list. Push it first so
    // the worker cannot finish its PENDING GET before the op exists.
    let missed = {
        let mut s = fake.state();
        let op = json!({ "id": "5000", "deviceId": "7", "status": "PENDING", "c8y_Restart": {} });
        s.operations.push(op.clone());
        op
    };
    fake.drop_realtime_clients();
    // 402 → immediate re-handshake (no Disconnected), then Connected + the PENDING op again.
    assert_eq!(next(&mut sub).await, RealtimeMsg::Connected);
    assert_eq!(op_id(&next(&mut sub).await), super::id_of(&missed).unwrap());
    assert_eq!(fake.realtime_clients(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realtime_reports_disconnect_and_recovers() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    fake.fail_next("POST", "/notification/realtime", 503);
    let mut sub = subscribe_device_with(&client, "7", fast()).unwrap();
    match next(&mut sub).await {
        RealtimeMsg::Disconnected(e) => assert!(e.contains("503"), "{e}"),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(next(&mut sub).await, RealtimeMsg::Connected);

    // Dropping the subscription stops the task and says goodbye to the server.
    drop(sub);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(fake.requests().iter().any(
        |r| r.path == "/notification/realtime" && r.json()[0]["channel"] == "/meta/disconnect"
    ));
    assert_eq!(fake.realtime_clients(), 0);
    let before = fake.requests().len();
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(fake.requests().len(), before, "no polling after drop");
}

/// No operation / connection message for `ms` (the open-alarm list of a reconnect is fine).
async fn expect_silence(sub: &mut RealtimeSubscription, ms: u64) {
    let until = tokio::time::Instant::now() + Duration::from_millis(ms);
    while let Ok(msg) = tokio::time::timeout_at(until, sub.recv()).await {
        if !matches!(msg, Some(RealtimeMsg::OpenAlarms { .. })) {
            panic!("unexpected realtime message {msg:?}");
        }
    }
}

async fn expect_closed(sub: &mut RealtimeSubscription) {
    let end = tokio::time::timeout(Duration::from_secs(3), sub.recv())
        .await
        .expect("task ends");
    assert_eq!(end, None, "task must stop");
}

fn realtime_posts(fake: &FakeC8y, channel: &str) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.path == "/notification/realtime" && r.json()[0]["channel"] == channel)
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn long_poll_outlives_the_client_timeout() {
    let fake = FakeC8y::start().await;
    // REST default 200 ms; the connect is held 300 ms. Only the per-request timeout
    // (poll_timeout + grace = 1 s) keeps it alive.
    let client = fake.client_with_timeout(Duration::from_millis(200));
    let mut sub = subscribe_device_with(&client, "7", fast()).unwrap();
    assert_eq!(next(&mut sub).await, RealtimeMsg::Connected);
    expect_silence(&mut sub, 1500).await; // no Disconnected
    assert_eq!(realtime_posts(&fake, "/meta/handshake"), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_connect_timeouts_in_a_row_are_a_lost_connection() {
    let fake = FakeC8y::start().await;
    {
        let mut s = fake.state();
        s.ignore_advice_timeout = true;
        s.connect_hold = Duration::from_millis(600);
    }
    let client = fake.client();
    let config = RealtimeConfig {
        poll_timeout: Duration::from_millis(100),
        poll_grace: Duration::from_millis(100),
        ..fast()
    };
    let mut sub = subscribe_device_with(&client, "7", config).unwrap();
    assert_eq!(next(&mut sub).await, RealtimeMsg::Connected);
    assert_eq!(
        next(&mut sub).await,
        RealtimeMsg::Disconnected("connect: timed out".into())
    );
    assert_eq!(realtime_posts(&fake, "/meta/connect"), 2, "two polls");
    assert!(realtime_posts(&fake, "/meta/disconnect") >= 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unauthorized_realtime_reports_once_and_stops() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let mut sub = subscribe_device_with(&client, "7", fast()).unwrap();
    assert_eq!(next(&mut sub).await, RealtimeMsg::Connected);
    fake.state().password = "rotated".into();
    match next(&mut sub).await {
        RealtimeMsg::Disconnected(e) => assert!(e.contains("401"), "{e}"),
        other => panic!("unexpected {other:?}"),
    }
    expect_closed(&mut sub).await;
    let before = fake.requests().len();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(fake.requests().len(), before, "no retries after 401");
    assert_eq!(fake.sink.expired.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forbidden_realtime_stops_with_a_clear_error() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    fake.fail_next("POST", "/notification/realtime", 403);
    let mut sub = subscribe_device_with(&client, "7", fast()).unwrap();
    match next(&mut sub).await {
        RealtimeMsg::Disconnected(e) => assert!(e.contains("permission denied"), "{e}"),
        other => panic!("unexpected {other:?}"),
    }
    expect_closed(&mut sub).await;
    assert!(fake.sink.expired.lock().unwrap().is_empty());
}

/// Alarm changes are optional: no read access to alarms (403 on the open-alarm list or on the
/// alarm subscription) must not stop operations.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forbidden_alarm_access_keeps_operations_flowing() {
    for subscribe_denied in [false, true] {
        let fake = FakeC8y::start().await;
        let client = fake.client();
        if subscribe_denied {
            // handshake, subscribe operations, then the alarm subscription.
            fake.fail_after("POST", "/notification/realtime", 2, 403);
        } else {
            fake.fail_next("GET", "/alarm/alarms", 403);
        }
        let mut sub = subscribe_device_with(&client, "7", fast()).unwrap();
        assert_eq!(next(&mut sub).await, RealtimeMsg::Connected);
        let live = fake.create_operation("7", json!({ "c8y_Restart": {} }));
        let msg = next(&mut sub).await;
        assert!(
            matches!(
                msg,
                RealtimeMsg::Operation(_) | RealtimeMsg::PendingOperation(_)
            ),
            "{msg:?}"
        );
        assert_eq!(op_id(&msg), super::id_of(&live).unwrap());
    }
}

/// A transient failure of the open-alarm list is retried once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_alarm_list_is_retried_once() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    fake.fail_next("GET", "/alarm/alarms", 503);
    let mut sub = subscribe_device_with(&client, "7", fast()).unwrap();
    assert_eq!(next_any(&mut sub).await, RealtimeMsg::Connected);
    assert!(matches!(
        next_any(&mut sub).await,
        RealtimeMsg::OpenAlarms { .. }
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn endless_unknown_client_is_backed_off() {
    let fake = FakeC8y::start().await;
    fake.state().always_unknown_client = true;
    let client = fake.client();
    let mut sub = subscribe_device_with(&client, "7", fast()).unwrap();
    let _ = next(&mut sub).await; // Connected (then 402 on every connect)
    // Drain for 1 s so the bounded channel never blocks the worker.
    let until = tokio::time::Instant::now() + Duration::from_secs(1);
    while tokio::time::timeout_at(until, sub.recv()).await.is_ok() {}
    let handshakes = realtime_posts(&fake, "/meta/handshake");
    // First re-handshake is immediate, then 50, 100, 200, 200 ms … (~7 in 1 s). Without
    // backoff this would be hundreds.
    assert!(
        (2..=12).contains(&handshakes),
        "{handshakes} handshakes in 1 s"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realtime_does_not_redeliver_known_operations() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let op = fake.create_operation("7", json!({ "c8y_Restart": {} }));
    let mut sub = subscribe_device_with(&client, "7", fast()).unwrap();
    assert_eq!(next(&mut sub).await, RealtimeMsg::Connected);
    assert_eq!(op_id(&next(&mut sub).await), super::id_of(&op).unwrap());

    // Re-handshake: the PENDING list returns the same op, which must be dropped.
    fake.drop_realtime_clients();
    assert_eq!(next(&mut sub).await, RealtimeMsg::Connected);
    expect_silence(&mut sub, 500).await;
}

/// D9: the alarm channel delivers status changes made in Cumulocity, and every (re)connect
/// delivers the open alarms, so a clear made while the connection was down is noticed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realtime_delivers_alarm_changes_and_open_alarms_on_every_connect() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    let a = client
        .create_alarm(
            &json!({ "type": "lw_x", "text": "x", "severity": "MAJOR", "source": { "id": "7" } }),
        )
        .await
        .unwrap();
    let id = super::id_of(&a).unwrap();
    let mut sub = subscribe_device_with(&client, "7", fast()).unwrap();
    assert_eq!(next_any(&mut sub).await, RealtimeMsg::Connected);
    match next_any(&mut sub).await {
        RealtimeMsg::OpenAlarms { alarms, .. } => {
            assert_eq!(alarms.len(), 1);
            assert_eq!(alarms[0]["id"], id.as_str());
        }
        other => panic!("unexpected {other:?}"),
    }
    let subs: Vec<Value> = fake
        .requests()
        .iter()
        .filter(|r| {
            r.path == "/notification/realtime" && r.json()[0]["channel"] == "/meta/subscribe"
        })
        .map(|r| r.json()[0]["subscription"].clone())
        .collect();
    assert_eq!(subs, vec![json!("/operations/7"), json!("/alarms/7")]);

    fake.set_alarm_status(&id, "ACKNOWLEDGED", true);
    match next_any(&mut sub).await {
        RealtimeMsg::Alarm(a) => {
            assert_eq!(a["id"], id.as_str());
            assert_eq!(a["status"], "ACKNOWLEDGED");
        }
        other => panic!("unexpected {other:?}"),
    }

    // Cleared while nobody listens; the reconnect's open-alarm list no longer has it.
    fake.set_alarm_status(&id, "CLEARED", false);
    fake.drop_realtime_clients();
    assert_eq!(next_any(&mut sub).await, RealtimeMsg::Connected);
    match next_any(&mut sub).await {
        RealtimeMsg::OpenAlarms { alarms, .. } => assert!(alarms.is_empty(), "{alarms:?}"),
        other => panic!("unexpected {other:?}"),
    }
}

/// The client measures how far the server's clock is off from the `Date` header (the session's
/// "Simulator was offline" rule compares `creationTime` with the server's clock).
#[tokio::test]
async fn clock_skew_comes_from_the_date_header() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    fake.set_clock_offset_secs(300);
    client.current_user().await.unwrap();
    let skew = client
        .clock_skew()
        .load(std::sync::atomic::Ordering::Relaxed);
    assert!((298_000..=302_000).contains(&skew), "{skew}");
    let server_now = super::server_now(&client.clock_skew());
    let ahead = (server_now - chrono::Utc::now()).num_seconds();
    assert!((298..=302).contains(&ahead), "{ahead}");
    fake.set_clock_offset_secs(-120);
    client.current_user().await.unwrap();
    let skew = client
        .clock_skew()
        .load(std::sync::atomic::Ordering::Relaxed);
    assert!((-122_000..=-118_000).contains(&skew), "{skew}");
}

/// The fake's `revert`, `childAssets` and `withParents` support (used by the engine tests).
#[tokio::test]
async fn fake_supports_revert_child_assets_and_parents() {
    let fake = FakeC8y::start().await;
    let client = fake.client();
    for (t, v) in [
        ("2026-10-12T10:00:00Z", 1),
        ("2026-10-12T12:00:00Z", 3),
        ("2026-10-12T11:00:00Z", 2),
    ] {
        client
            .create_event(
                &json!({ "type": "x", "text": v.to_string(), "time": t, "source": { "id": "7" } }),
            )
            .await
            .unwrap();
    }
    let newest = client.query_events("7", Some("x"), 2).await.unwrap();
    assert_eq!(
        newest
            .iter()
            .map(|e| e["text"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["3", "2"],
        "newest first"
    );
    let group = fake.insert_managed_object(json!({ "name": "G", "c8y_IsDeviceGroup": {} }));
    let dev = fake.insert_managed_object(json!({ "name": "D" }));
    let _: serde_json::Value = client
        .post_json(
            &format!("/inventory/managedObjects/{group}/childAssets"),
            &json!({ "managedObject": { "id": dev } }),
        )
        .await
        .unwrap();
    let with: serde_json::Value = client
        .get_json(&format!("/inventory/managedObjects/{dev}?withParents=true"))
        .await
        .unwrap();
    assert_eq!(
        with["assetParents"]["references"][0]["managedObject"]["name"],
        "G"
    );
    let without: serde_json::Value = client
        .get_json(&format!("/inventory/managedObjects/{dev}"))
        .await
        .unwrap();
    assert!(without.get("assetParents").is_none());
}
