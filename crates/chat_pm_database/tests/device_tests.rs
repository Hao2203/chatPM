use chat_pm_database::ChatDb;
use chat_pm_session::session::SessionId;
use chat_pm_sync::DeviceId;

#[test]
fn test_device_registration_and_query() {
    let db = ChatDb::open_in_memory().unwrap();

    let device_id = DeviceId::generate();
    db.register_device(device_id, Some("测试设备")).unwrap();

    let record = db.get_device(device_id).unwrap().expect("设备应该存在");
    assert_eq!(record.device_id, device_id);
    assert_eq!(record.name.as_deref(), Some("测试设备"));

    // 重复注册应该幂等
    db.register_device(device_id, Some("新名称")).unwrap();
    let record = db.get_device(device_id).unwrap().expect("设备应该存在");
    assert_eq!(record.name.as_deref(), Some("测试设备"));

    // 列出设备
    let devices = db.list_devices().unwrap();
    assert_eq!(devices.len(), 1);
}

#[test]
fn test_turn_local_seq_no() {
    let db = ChatDb::open_in_memory().unwrap();
    let device_id = DeviceId::generate();
    db.register_device(device_id, None).unwrap();

    let sid = SessionId::new();
    db.create_session(sid).unwrap();

    // 第一轮：seq_no = 0
    db.append_chat_turn(sid, "你好".into(), "你好！".into(), None, None, device_id)
        .unwrap();

    let turns = db.recent_turns(sid, 10).unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].local_seq_no, 0);
    assert_eq!(turns[0].device_id, Some(device_id));

    // 第二轮：seq_no = 1
    db.append_chat_turn(
        sid,
        "今天天气怎么样？".into(),
        "今天晴天。".into(),
        Some(100),
        Some(50),
        device_id,
    )
    .unwrap();

    let turns = db.recent_turns(sid, 10).unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].local_seq_no, 0);
    assert_eq!(turns[1].local_seq_no, 1);
    assert_eq!(turns[1].prompt_tokens, Some(100));
    assert_eq!(turns[1].completion_tokens, Some(50));
}

#[test]
fn test_local_seq_no_per_session() {
    let db = ChatDb::open_in_memory().unwrap();
    let device_id = DeviceId::generate();
    db.register_device(device_id, None).unwrap();

    let sid_a = SessionId::new();
    let sid_b = SessionId::new();
    db.create_session(sid_a).unwrap();
    db.create_session(sid_b).unwrap();

    // Session A: 2 turns
    db.append_chat_turn(sid_a, "a1".into(), "r1".into(), None, None, device_id)
        .unwrap();
    db.append_chat_turn(sid_a, "a2".into(), "r2".into(), None, None, device_id)
        .unwrap();

    // Session B: 1 turn
    db.append_chat_turn(sid_b, "b1".into(), "r3".into(), None, None, device_id)
        .unwrap();

    // 每个 session 独立计数，从 0 开始
    let turns_a = db.recent_turns(sid_a, 10).unwrap();
    assert_eq!(turns_a[0].local_seq_no, 0);
    assert_eq!(turns_a[1].local_seq_no, 1);

    let turns_b = db.recent_turns(sid_b, 10).unwrap();
    assert_eq!(turns_b[0].local_seq_no, 0);
}
