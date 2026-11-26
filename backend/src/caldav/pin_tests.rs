//! The DAV client against local servers: credentials reach only the account's server.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};

use super::{CalDavClient, Service};
use crate::carddav;

/// A request a test server saw: method, path, and whether it carried credentials.
type Seen = (String, String, bool);

/// A local server answering with `answer(method, path)` (status, headers, body), recording what it saw.
async fn server(
    answer: impl Fn(&str, &str) -> (u16, Vec<(&'static str, String)>, String) + Send + Sync + 'static,
) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let answer = Arc::new(answer);
    let app = axum::Router::new().fallback(move |req: Request<Body>| {
        let log = log.clone();
        let answer = answer.clone();
        async move {
            let method = req.method().to_string();
            let path = req.uri().path().to_owned();
            let auth = req.headers().contains_key("authorization");
            log.lock()
                .unwrap()
                .push((method.clone(), path.clone(), auth));
            let (status, headers, body) = answer(&method, &path);
            let mut resp = Response::builder().status(StatusCode::from_u16(status).unwrap());
            for (k, v) in headers {
                resp = resp.header(k, v);
            }
            resp.body(Body::from(body)).unwrap()
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, seen)
}

fn client(base: &str) -> CalDavClient {
    let _ = rustls::crypto::ring::default_provider().install_default();
    CalDavClient::new(base.to_owned(), "alice".into(), "secret".into()).unwrap()
}

fn multistatus(responses: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">{responses}</d:multistatus>"#
    )
}

/// The other host: whatever it is asked, it answers 200, and records it.
async fn other() -> (String, Arc<Mutex<Vec<Seen>>>) {
    server(|_, _| (200, vec![], multistatus(""))).await
}

#[tokio::test]
async fn a_redirect_to_another_origin_is_refused_without_credentials_sent() {
    let (evil, evil_seen) = other().await;
    let target = format!("{evil}/steal/");
    let (base, seen) = server(move |_, path| match path {
        "/cal/" => (301, vec![("location", target.clone())], String::new()),
        _ => (404, vec![], String::new()),
    })
    .await;
    let c = client(&base);
    let e = c
        .propfind(&format!("{base}/cal/"), "0", "<propfind/>")
        .await
        .unwrap_err();
    assert!(e.to_string().contains("not the account's server"), "{e}");
    let e = c
        .put_event(
            &format!("{base}/cal/x.ics"),
            "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n",
        )
        .await;
    assert!(e.is_err());
    // PUT /cal/x.ics was answered 404 by the account's server; the PROPFIND's redirect not followed
    assert!(
        evil_seen.lock().unwrap().is_empty(),
        "{:?}",
        evil_seen.lock().unwrap()
    );
    assert!(seen.lock().unwrap().iter().all(|(_, _, auth)| *auth));
}

#[tokio::test]
async fn a_relative_redirect_on_the_same_origin_is_followed() {
    let (base, seen) = server(|method, path| match (method, path) {
        ("PROPFIND", "/old/") => (308, vec![("location", "/new/".into())], String::new()),
        ("PROPFIND", "/new/") => (
            207,
            vec![],
            multistatus("<d:response><d:href>/new/</d:href></d:response>"),
        ),
        ("DELETE", "/new/a.ics") => (307, vec![("location", "b.ics".into())], String::new()),
        ("DELETE", "/new/b.ics") => (204, vec![], String::new()),
        _ => (404, vec![], String::new()),
    })
    .await;
    let c = client(&base);
    let xml = c
        .propfind(&format!("{base}/old/"), "0", "<propfind/>")
        .await
        .unwrap();
    assert!(xml.contains("/new/"));
    c.delete_event(&format!("{base}/new/a.ics")).await.unwrap();
    let seen = seen.lock().unwrap();
    let paths: Vec<&str> = seen.iter().map(|(_, p, _)| p.as_str()).collect();
    assert_eq!(paths, ["/old/", "/new/", "/new/a.ics", "/new/b.ics"]);
}

#[tokio::test]
async fn a_stored_href_on_another_origin_is_never_requested() {
    let (evil, evil_seen) = other().await;
    let (base, _) = server(|_, _| (404, vec![], String::new())).await;
    let c = client(&base);
    let href = format!("{evil}/cal/x.ics");
    let e = c
        .put_event(&href, "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n")
        .await
        .unwrap_err();
    assert!(e.to_string().contains("refused"), "{e}");
    assert!(c.delete_event(&href).await.is_err());
    assert!(c.get_event(&href).await.is_err());
    assert!(
        c.register_push(
            &format!("{evil}/cal/"),
            "https://caley.example/push",
            "k",
            "a"
        )
        .await
        .is_err()
    );
    assert!(
        carddav::put_contact(
            &c,
            &format!("{evil}/ab/x.vcf"),
            "BEGIN:VCARD\r\nEND:VCARD\r\n",
            true
        )
        .await
        .is_err()
    );
    assert!(
        carddav::delete_contact(&c, &format!("{evil}/ab/x.vcf"))
            .await
            .is_err()
    );
    assert!(evil_seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn well_known_discovery_moves_only_over_https() {
    // same origin, relative: discovery starts there
    let (base, _) = server(|_, path| match path {
        "/.well-known/caldav" => (301, vec![("location", "/dav/".into())], String::new()),
        _ => (404, vec![], String::new()),
    })
    .await;
    let c = client(&base);
    assert_eq!(
        c.start_url_for(Service::CalDav).await,
        format!("{base}/dav")
    );

    // another origin over http: not adopted, and nothing sent there
    let (evil, evil_seen) = other().await;
    let target = format!("{evil}/dav/");
    let (base, _) = server(move |_, path| match path {
        "/.well-known/caldav" | "/.well-known/carddav" => {
            (301, vec![("location", target.clone())], String::new())
        }
        _ => (404, vec![], String::new()),
    })
    .await;
    let c = client(&base);
    assert_eq!(c.start_url_for(Service::CalDav).await, base);
    assert_eq!(c.start_url_for(Service::CardDav).await, base);
    assert!(c.calendars().await.is_err());
    assert!(c.get_event(&format!("{evil}/dav/x.ics")).await.is_err());
    assert!(evil_seen.lock().unwrap().is_empty());
}

/// Discovery over https to another host (seeded: the test servers speak http) pins that host: its hrefs
/// are requested, a third host's are not.
#[tokio::test]
async fn the_origin_well_known_reached_is_the_accounts() {
    let (base, _) = server(|_, _| (404, vec![], String::new())).await;
    let c = client(&base);
    c.well_known[Service::CalDav as usize]
        .set(Some(
            url::Url::parse("https://dav.provider.example/dav/").unwrap(),
        ))
        .unwrap();
    let servers = c
        .servers_for(&url::Url::parse("https://dav.provider.example/cal/x.ics").unwrap())
        .await;
    assert!(
        servers
            .iter()
            .any(|s| s.host_str() == Some("dav.provider.example"))
    );
    assert!(
        super::pin::redirect_target(
            &url::Url::parse("https://dav.provider.example/cal/").unwrap(),
            "https://third.example/",
            &servers,
            "PROPFIND"
        )
        .is_err()
    );
}

#[tokio::test]
async fn an_absolute_href_on_another_origin_in_a_multistatus_is_skipped() {
    let (evil, evil_seen) = other().await;
    let foreign = format!("{evil}/home/evil/");
    let (base, _) = server(move |method, path| match (method, path) {
        ("PROPFIND", "/") => (
            207,
            vec![],
            multistatus("<d:response><d:href>/</d:href><d:propstat><d:prop><d:current-user-principal><d:href>/p/</d:href></d:current-user-principal></d:prop></d:propstat></d:response>"),
        ),
        ("PROPFIND", "/p/") => (
            207,
            vec![],
            multistatus("<d:response><d:href>/p/</d:href><d:propstat><d:prop><c:calendar-home-set><d:href>/home/</d:href></c:calendar-home-set></d:prop></d:propstat></d:response>"),
        ),
        ("PROPFIND", "/home/") => (
            207,
            vec![],
            multistatus(&format!(
                "<d:response><d:href>/home/</d:href></d:response>\
                 <d:response><d:href>/home/work/</d:href><d:propstat><d:prop><d:displayname>Work</d:displayname></d:prop></d:propstat></d:response>\
                 <d:response><d:href>{foreign}</d:href><d:propstat><d:prop><d:displayname>Evil</d:displayname></d:prop></d:propstat></d:response>"
            )),
        ),
        ("REPORT", "/home/work/") => (
            207,
            vec![],
            multistatus(&format!(
                "<d:sync-token>t2</d:sync-token>\
                 <d:response><d:href>/home/work/a.ics</d:href><d:propstat><d:prop><d:getetag>\"1\"</d:getetag><c:calendar-data>BEGIN:VCALENDAR\r\nEND:VCALENDAR</c:calendar-data></d:prop></d:propstat></d:response>\
                 <d:response><d:href>{foreign}b.ics</d:href><d:propstat><d:prop><d:getetag>\"1\"</d:getetag><c:calendar-data>BEGIN:VCALENDAR\r\nEND:VCALENDAR</c:calendar-data></d:prop></d:propstat></d:response>\
                 <d:response><d:href>{foreign}c.ics</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>"
            )),
        ),
        _ => (404, vec![], String::new()),
    })
    .await;
    let c = client(&base);
    let calendars = c.calendars().await.unwrap();
    let hrefs: Vec<&str> = calendars.iter().map(|c| c.href.as_str()).collect();
    assert_eq!(hrefs, [format!("{base}/home/work/")]);
    let work = format!("{base}/home/work/");
    let sync = c.sync_collection(&work, Some("t1")).await.unwrap();
    let changed: Vec<&str> = sync.changed.iter().map(|(h, _, _)| h.as_str()).collect();
    assert_eq!(changed, [format!("{base}/home/work/a.ics")]);
    assert!(sync.deleted.is_empty());
    let all = c.fetch_all_raw(&work).await.unwrap();
    assert_eq!(all.len(), 1);
    assert!(evil_seen.lock().unwrap().is_empty());
}
