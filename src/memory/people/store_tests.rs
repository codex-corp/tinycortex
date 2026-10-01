use super::*;

#[tokio::test]
async fn insert_list_and_lookup_round_trip() {
    let s = PeopleStore::open_in_memory().unwrap();
    let now = Utc::now();
    let p = Person {
        id: PersonId::new(),
        display_name: Some("Sarah Lee".into()),
        primary_email: Some("sarah@example.com".into()),
        primary_phone: None,
        handles: vec![],
        created_at: now,
        updated_at: now,
    };
    s.insert_person(
        &p,
        &[
            Handle::Email("Sarah@Example.com".into()),
            Handle::DisplayName("Sarah Lee".into()),
        ],
    )
    .await
    .unwrap();

    let got = s
        .lookup(&Handle::Email("sarah@example.com".into()))
        .await
        .unwrap();
    assert_eq!(got, Some(p.id));

    let list = s.list().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].handles.len(), 2);
}

#[tokio::test]
async fn interactions_round_trip() {
    let s = PeopleStore::open_in_memory().unwrap();
    let now = Utc::now();
    let pid = PersonId::new();
    let p = Person {
        id: pid,
        display_name: Some("X".into()),
        primary_email: None,
        primary_phone: None,
        handles: vec![],
        created_at: now,
        updated_at: now,
    };
    s.insert_person(&p, &[]).await.unwrap();
    s.record_interaction(Interaction {
        person_id: pid,
        ts: now,
        is_outbound: true,
        length: 100,
    })
    .await
    .unwrap();
    s.record_interaction(Interaction {
        person_id: pid,
        ts: now,
        is_outbound: false,
        length: 50,
    })
    .await
    .unwrap();
    let ints = s.interactions_for(pid).await.unwrap();
    assert_eq!(ints.len(), 2);
}
