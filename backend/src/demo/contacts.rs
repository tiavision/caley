use sqlx::SqlitePool;

use super::{DEMO_ADDRESSBOOK_HREF, DEMO_CALDAV_ACCOUNT};

pub(super) async fn seed_addressbook(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO cached_addressbooks
            (account_id, href, name, sync_token, last_synced)
         VALUES (?, ?, 'Contacts', 'demo-token', strftime('%s','now'))",
    )
    .bind(DEMO_CALDAV_ACCOUNT)
    .bind(DEMO_ADDRESSBOOK_HREF)
    .execute(pool)
    .await?;
    Ok(())
}

struct ContactSeed {
    uid: &'static str,
    fn_name: &'static str,
    org: &'static str,
    emails: &'static str,
    phones: &'static str,
    vcard: &'static str,
}

pub(super) async fn seed_contacts(pool: &SqlitePool) -> anyhow::Result<()> {
    let contacts: &[ContactSeed] = &[
        ContactSeed { uid: "demo-alice-brennan", fn_name: "Alice Brennan", org: "Acme Corp", emails: r#"[{"value":"alice@acme.example","type_label":"work"}]"#, phones: r#"[{"value":"+1 555 010 1001","type_label":"work"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-alice-brennan\r\nFN:Alice Brennan\r\nN:Brennan;Alice;;;\r\nORG:Acme Corp\r\nEMAIL;TYPE=WORK:alice@acme.example\r\nTEL;TYPE=WORK:+1 555 010 1001\r\nEND:VCARD" },
        ContactSeed { uid: "demo-andrew-miller", fn_name: "Andrew Miller", org: "Acme Corp", emails: r#"[{"value":"a.miller@acme.example","type_label":"work"},{"value":"andrew.miller@personal.example","type_label":"home"}]"#, phones: r#"[{"value":"+1 555 010 1002","type_label":"work"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-andrew-miller\r\nFN:Andrew Miller\r\nN:Miller;Andrew;;;\r\nORG:Acme Corp\r\nEMAIL;TYPE=WORK:a.miller@acme.example\r\nEMAIL;TYPE=HOME:andrew.miller@personal.example\r\nTEL;TYPE=WORK:+1 555 010 1002\r\nEND:VCARD" },
        ContactSeed { uid: "demo-aisha-patel", fn_name: "Aisha Patel", org: "Design Studio", emails: r#"[{"value":"aisha@designstudio.example","type_label":"work"}]"#, phones: r#"[{"value":"+44 7700 900010","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-aisha-patel\r\nFN:Aisha Patel\r\nN:Patel;Aisha;;;\r\nORG:Design Studio\r\nEMAIL;TYPE=WORK:aisha@designstudio.example\r\nTEL;TYPE=CELL:+44 7700 900010\r\nEND:VCARD" },
        ContactSeed { uid: "demo-bob-chen", fn_name: "Bob Chen", org: "Freelance", emails: r#"[{"value":"bob.chen@example.net","type_label":"home"}]"#, phones: r#"[{"value":"+44 7700 900001","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-bob-chen\r\nFN:Bob Chen\r\nN:Chen;Bob;;;\r\nORG:Freelance\r\nEMAIL;TYPE=HOME:bob.chen@example.net\r\nTEL;TYPE=CELL:+44 7700 900001\r\nEND:VCARD" },
        ContactSeed { uid: "demo-barbara-williams", fn_name: "Barbara Williams", org: "Acme Corp", emails: r#"[{"value":"b.williams@acme.example","type_label":"work"}]"#, phones: r#"[{"value":"+1 555 010 2001","type_label":"work"},{"value":"+1 555 010 2099","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-barbara-williams\r\nFN:Barbara Williams\r\nN:Williams;Barbara;;;\r\nORG:Acme Corp\r\nEMAIL;TYPE=WORK:b.williams@acme.example\r\nTEL;TYPE=WORK:+1 555 010 2001\r\nTEL;TYPE=CELL:+1 555 010 2099\r\nEND:VCARD" },
        ContactSeed { uid: "demo-carla-diaz", fn_name: "Carla Diaz", org: "City Hospital", emails: r#"[{"value":"c.diaz@hospital.example","type_label":"work"}]"#, phones: r#"[{"value":"+1 555 020 2002","type_label":"work"},{"value":"+1 555 020 2010","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-carla-diaz\r\nFN:Carla Diaz\r\nN:Diaz;Carla;;;\r\nORG:City Hospital\r\nEMAIL;TYPE=WORK:c.diaz@hospital.example\r\nTEL;TYPE=WORK:+1 555 020 2002\r\nTEL;TYPE=CELL:+1 555 020 2010\r\nEND:VCARD" },
        ContactSeed { uid: "demo-carlos-fernandez", fn_name: "Carlos Fernandez", org: "Startup Inc", emails: r#"[{"value":"carlos@startup.example","type_label":"work"}]"#, phones: r#"[{"value":"+34 612 345 678","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-carlos-fernandez\r\nFN:Carlos Fernandez\r\nN:Fernandez;Carlos;;;\r\nORG:Startup Inc\r\nEMAIL;TYPE=WORK:carlos@startup.example\r\nTEL;TYPE=CELL:+34 612 345 678\r\nEND:VCARD" },
        ContactSeed { uid: "demo-claire-nguyen", fn_name: "Claire Nguyen", org: "Marketing Agency", emails: r#"[{"value":"claire.nguyen@mktg.example","type_label":"work"},{"value":"claire.n@home.example","type_label":"home"}]"#, phones: r#"[{"value":"+1 555 020 3003","type_label":"work"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-claire-nguyen\r\nFN:Claire Nguyen\r\nN:Nguyen;Claire;;;\r\nORG:Marketing Agency\r\nEMAIL;TYPE=WORK:claire.nguyen@mktg.example\r\nEMAIL;TYPE=HOME:claire.n@home.example\r\nTEL;TYPE=WORK:+1 555 020 3003\r\nEND:VCARD" },
        ContactSeed { uid: "demo-david-kim", fn_name: "David Kim", org: "Startup Inc", emails: r#"[{"value":"david@startup.example","type_label":"work"},{"value":"d.kim@personal.example","type_label":"home"}]"#, phones: r#"[{"value":"+82 10 1234 5678","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-david-kim\r\nFN:David Kim\r\nN:Kim;David;;;\r\nORG:Startup Inc\r\nEMAIL;TYPE=WORK:david@startup.example\r\nEMAIL;TYPE=HOME:d.kim@personal.example\r\nTEL;TYPE=CELL:+82 10 1234 5678\r\nEND:VCARD" },
        ContactSeed { uid: "demo-diana-reynolds", fn_name: "Diana Reynolds", org: "Reynolds Legal", emails: r#"[{"value":"diana@reynoldslegal.example","type_label":"work"}]"#, phones: r#"[{"value":"+1 555 030 1001","type_label":"work"},{"value":"+1 555 030 1099","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-diana-reynolds\r\nFN:Diana Reynolds\r\nN:Reynolds;Diana;;;\r\nORG:Reynolds Legal\r\nEMAIL;TYPE=WORK:diana@reynoldslegal.example\r\nTEL;TYPE=WORK:+1 555 030 1001\r\nTEL;TYPE=CELL:+1 555 030 1099\r\nEND:VCARD" },
        ContactSeed { uid: "demo-emma-johansson", fn_name: "Emma Johansson", org: "Design Studio", emails: r#"[{"value":"emma@designstudio.example","type_label":"work"}]"#, phones: r#"[{"value":"+46 70 123 45 67","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-emma-johansson\r\nFN:Emma Johansson\r\nN:Johansson;Emma;;;\r\nORG:Design Studio\r\nEMAIL;TYPE=WORK:emma@designstudio.example\r\nTEL;TYPE=CELL:+46 70 123 45 67\r\nEND:VCARD" },
        ContactSeed { uid: "demo-ethan-brooks", fn_name: "Ethan Brooks", org: "Acme Corp", emails: r#"[{"value":"e.brooks@acme.example","type_label":"work"}]"#, phones: r#"[{"value":"+1 555 040 1001","type_label":"work"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-ethan-brooks\r\nFN:Ethan Brooks\r\nN:Brooks;Ethan;;;\r\nORG:Acme Corp\r\nEMAIL;TYPE=WORK:e.brooks@acme.example\r\nTEL;TYPE=WORK:+1 555 040 1001\r\nEND:VCARD" },
        ContactSeed { uid: "demo-felix-mueller", fn_name: "Felix Mueller", org: "Tech GmbH", emails: r#"[{"value":"felix.mueller@tech.example","type_label":"work"}]"#, phones: r#"[{"value":"+49 170 123 4567","type_label":"work"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-felix-mueller\r\nFN:Felix Mueller\r\nN:Mueller;Felix;;;\r\nORG:Tech GmbH\r\nEMAIL;TYPE=WORK:felix.mueller@tech.example\r\nTEL;TYPE=WORK:+49 170 123 4567\r\nEND:VCARD" },
        ContactSeed { uid: "demo-fatima-al-rashid", fn_name: "Fatima Al-Rashid", org: "Global Consulting", emails: r#"[{"value":"fatima@globalconsulting.example","type_label":"work"}]"#, phones: r#"[{"value":"+971 50 123 4567","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-fatima-al-rashid\r\nFN:Fatima Al-Rashid\r\nN:Al-Rashid;Fatima;;;\r\nORG:Global Consulting\r\nEMAIL;TYPE=WORK:fatima@globalconsulting.example\r\nTEL;TYPE=CELL:+971 50 123 4567\r\nEND:VCARD" },
        ContactSeed { uid: "demo-grace-okafor", fn_name: "Grace Okafor", org: "NGO International", emails: r#"[{"value":"grace.okafor@ngo.example","type_label":"work"}]"#, phones: r#"[{"value":"+234 802 345 6789","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-grace-okafor\r\nFN:Grace Okafor\r\nN:Okafor;Grace;;;\r\nORG:NGO International\r\nEMAIL;TYPE=WORK:grace.okafor@ngo.example\r\nTEL;TYPE=CELL:+234 802 345 6789\r\nEND:VCARD" },
        ContactSeed { uid: "demo-george-thompson", fn_name: "George Thompson", org: "Thompson Consulting", emails: r#"[{"value":"george@tconsulting.example","type_label":"work"},{"value":"g.thompson@home.example","type_label":"home"}]"#, phones: r#"[{"value":"+1 555 050 1001","type_label":"work"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-george-thompson\r\nFN:George Thompson\r\nN:Thompson;George;;;\r\nORG:Thompson Consulting\r\nEMAIL;TYPE=WORK:george@tconsulting.example\r\nEMAIL;TYPE=HOME:g.thompson@home.example\r\nTEL;TYPE=WORK:+1 555 050 1001\r\nEND:VCARD" },
        ContactSeed { uid: "demo-henry-park", fn_name: "Henry Park", org: "", emails: r#"[{"value":"henry.park@personal.example","type_label":"home"}]"#, phones: r#"[{"value":"+1 555 060 1001","type_label":"home"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-henry-park\r\nFN:Henry Park\r\nN:Park;Henry;;;\r\nEMAIL;TYPE=HOME:henry.park@personal.example\r\nTEL;TYPE=HOME:+1 555 060 1001\r\nEND:VCARD" },
        ContactSeed { uid: "demo-hannah-schmidt", fn_name: "Hannah Schmidt", org: "Acme Corp", emails: r#"[{"value":"h.schmidt@acme.example","type_label":"work"}]"#, phones: r#"[{"value":"+49 151 234 56789","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-hannah-schmidt\r\nFN:Hannah Schmidt\r\nN:Schmidt;Hannah;;;\r\nORG:Acme Corp\r\nEMAIL;TYPE=WORK:h.schmidt@acme.example\r\nTEL;TYPE=CELL:+49 151 234 56789\r\nEND:VCARD" },
        ContactSeed { uid: "demo-isabella-romano", fn_name: "Isabella Romano", org: "PR Agency", emails: r#"[{"value":"i.romano@pragency.example","type_label":"work"}]"#, phones: r#"[{"value":"+39 347 123 4567","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-isabella-romano\r\nFN:Isabella Romano\r\nN:Romano;Isabella;;;\r\nORG:PR Agency\r\nEMAIL;TYPE=WORK:i.romano@pragency.example\r\nTEL;TYPE=CELL:+39 347 123 4567\r\nEND:VCARD" },
        ContactSeed { uid: "demo-james-wilson", fn_name: "James Wilson", org: "City Bank", emails: r#"[{"value":"james.wilson@citybank.example","type_label":"work"}]"#, phones: r#"[{"value":"+1 555 070 1001","type_label":"work"},{"value":"+1 555 070 9999","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-james-wilson\r\nFN:James Wilson\r\nN:Wilson;James;;;\r\nORG:City Bank\r\nEMAIL;TYPE=WORK:james.wilson@citybank.example\r\nTEL;TYPE=WORK:+1 555 070 1001\r\nTEL;TYPE=CELL:+1 555 070 9999\r\nEND:VCARD" },
        ContactSeed { uid: "demo-katherine-hayes", fn_name: "Katherine Hayes", org: "", emails: r#"[{"value":"kath.hayes@personal.example","type_label":"home"}]"#, phones: r#"[{"value":"+1 555 080 2002","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-katherine-hayes\r\nFN:Katherine Hayes\r\nN:Hayes;Katherine;;;\r\nEMAIL;TYPE=HOME:kath.hayes@personal.example\r\nTEL;TYPE=CELL:+1 555 080 2002\r\nEND:VCARD" },
        ContactSeed { uid: "demo-liam-obrien", fn_name: "Liam O'Brien", org: "Tech GmbH", emails: r#"[{"value":"liam.obrien@tech.example","type_label":"work"}]"#, phones: r#"[{"value":"+353 87 123 4567","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-liam-obrien\r\nFN:Liam O'Brien\r\nN:O'Brien;Liam;;;\r\nORG:Tech GmbH\r\nEMAIL;TYPE=WORK:liam.obrien@tech.example\r\nTEL;TYPE=CELL:+353 87 123 4567\r\nEND:VCARD" },
        ContactSeed { uid: "demo-maria-santos", fn_name: "Maria Santos", org: "City Hospital", emails: r#"[{"value":"m.santos@hospital.example","type_label":"work"}]"#, phones: r#"[{"value":"+55 11 9 1234 5678","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-maria-santos\r\nFN:Maria Santos\r\nN:Santos;Maria;;;\r\nORG:City Hospital\r\nEMAIL;TYPE=WORK:m.santos@hospital.example\r\nTEL;TYPE=CELL:+55 11 9 1234 5678\r\nEND:VCARD" },
        ContactSeed { uid: "demo-michael-chang", fn_name: "Michael Chang", org: "Startup Inc", emails: r#"[{"value":"michael@startup.example","type_label":"work"},{"value":"m.chang@home.example","type_label":"home"}]"#, phones: r#"[{"value":"+1 555 090 1001","type_label":"work"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-michael-chang\r\nFN:Michael Chang\r\nN:Chang;Michael;;;\r\nORG:Startup Inc\r\nEMAIL;TYPE=WORK:michael@startup.example\r\nEMAIL;TYPE=HOME:m.chang@home.example\r\nTEL;TYPE=WORK:+1 555 090 1001\r\nEND:VCARD" },
        ContactSeed { uid: "demo-nina-petrov", fn_name: "Nina Petrov", org: "Global Consulting", emails: r#"[{"value":"nina.petrov@globalconsulting.example","type_label":"work"}]"#, phones: r#"[{"value":"+7 916 123 45 67","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-nina-petrov\r\nFN:Nina Petrov\r\nN:Petrov;Nina;;;\r\nORG:Global Consulting\r\nEMAIL;TYPE=WORK:nina.petrov@globalconsulting.example\r\nTEL;TYPE=CELL:+7 916 123 45 67\r\nEND:VCARD" },
        ContactSeed { uid: "demo-oliver-grant", fn_name: "Oliver Grant", org: "Acme Corp", emails: r#"[{"value":"o.grant@acme.example","type_label":"work"}]"#, phones: r#"[{"value":"+44 7700 900200","type_label":"work"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-oliver-grant\r\nFN:Oliver Grant\r\nN:Grant;Oliver;;;\r\nORG:Acme Corp\r\nEMAIL;TYPE=WORK:o.grant@acme.example\r\nTEL;TYPE=WORK:+44 7700 900200\r\nEND:VCARD" },
        ContactSeed { uid: "demo-rafael-silva", fn_name: "Rafael Silva", org: "Design Studio", emails: r#"[{"value":"rafael@designstudio.example","type_label":"work"}]"#, phones: r#"[{"value":"+55 21 9 8765 4321","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-rafael-silva\r\nFN:Rafael Silva\r\nN:Silva;Rafael;;;\r\nORG:Design Studio\r\nEMAIL;TYPE=WORK:rafael@designstudio.example\r\nTEL;TYPE=CELL:+55 21 9 8765 4321\r\nEND:VCARD" },
        ContactSeed { uid: "demo-sarah-cohen", fn_name: "Sarah Cohen", org: "", emails: r#"[{"value":"sarah.cohen@personal.example","type_label":"home"}]"#, phones: r#"[{"value":"+1 555 100 1001","type_label":"home"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-sarah-cohen\r\nFN:Sarah Cohen\r\nN:Cohen;Sarah;;;\r\nEMAIL;TYPE=HOME:sarah.cohen@personal.example\r\nTEL;TYPE=HOME:+1 555 100 1001\r\nEND:VCARD" },
        ContactSeed { uid: "demo-sophie-laurent", fn_name: "Sophie Laurent", org: "Marketing Agency", emails: r#"[{"value":"sophie.laurent@mktg.example","type_label":"work"}]"#, phones: r#"[{"value":"+33 6 12 34 56 78","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-sophie-laurent\r\nFN:Sophie Laurent\r\nN:Laurent;Sophie;;;\r\nORG:Marketing Agency\r\nEMAIL;TYPE=WORK:sophie.laurent@mktg.example\r\nTEL;TYPE=CELL:+33 6 12 34 56 78\r\nEND:VCARD" },
        ContactSeed { uid: "demo-tomoki-yamada", fn_name: "Tomoki Yamada", org: "Tech GmbH", emails: r#"[{"value":"t.yamada@tech.example","type_label":"work"}]"#, phones: r#"[{"value":"+81 90 1234 5678","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-tomoki-yamada\r\nFN:Tomoki Yamada\r\nN:Yamada;Tomoki;;;\r\nORG:Tech GmbH\r\nEMAIL;TYPE=WORK:t.yamada@tech.example\r\nTEL;TYPE=CELL:+81 90 1234 5678\r\nEND:VCARD" },
        ContactSeed { uid: "demo-zara-ahmed", fn_name: "Zara Ahmed", org: "NGO International", emails: r#"[{"value":"zara.ahmed@ngo.example","type_label":"work"}]"#, phones: r#"[{"value":"+92 300 123 4567","type_label":"mobile"}]"#, vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-zara-ahmed\r\nFN:Zara Ahmed\r\nN:Ahmed;Zara;;;\r\nORG:NGO International\r\nEMAIL;TYPE=WORK:zara.ahmed@ngo.example\r\nTEL;TYPE=CELL:+92 300 123 4567\r\nEND:VCARD" },
    ];

    for c in contacts {
        sqlx::query(
            "INSERT OR IGNORE INTO cached_contacts
                (account_id, addressbook_href, uid, etag, href, raw_vcard, fn_name, emails, phones, org)
             VALUES (?, ?, ?, 'demo', ?, ?, ?, ?, ?, ?)",
        )
        .bind(DEMO_CALDAV_ACCOUNT)
        .bind(DEMO_ADDRESSBOOK_HREF)
        .bind(c.uid)
        .bind(format!("{}{}.vcf", DEMO_ADDRESSBOOK_HREF, c.uid))
        .bind(c.vcard)
        .bind(c.fn_name)
        .bind(c.emails)
        .bind(c.phones)
        .bind(c.org)
        .execute(pool)
        .await?;
    }
    Ok(())
}
