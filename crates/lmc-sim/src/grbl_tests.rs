use super::*;
use openlaser_types::VirtualClock;
fn read(b: &mut FakeGrbl) -> String {
    let mut v = Vec::new();
    b.read_available(&mut v, Duration::ZERO).unwrap_or_else(|e| panic!("{e}"));
    String::from_utf8(v).unwrap_or_else(|e| panic!("{e}"))
}
fn write(b: &mut FakeGrbl, s: &str) {
    b.write_all(s.as_bytes(), Duration::ZERO).unwrap_or_else(|e| panic!("{e}"));
}
#[test]
fn fake_grbl_streams_square() {
    let clock = Arc::new(VirtualClock::new());
    let mut b = FakeGrbl::new(clock.clone());
    assert!(read(&mut b).contains("Grbl 1.1h"));
    write(&mut b, "G21 G90\nG0 X0 Y0\nM3 S100\nG1 X10 Y0 F300\nG1 X10 Y10\nG1 X0 Y10\nG1 X0 Y0\nM5\n");
    assert_eq!(read(&mut b).matches("ok\r\n").count(), 8);
    assert_eq!(b.log().len(), 8);
    b.write_realtime(b'?').unwrap_or_else(|e| panic!("{e}"));
    assert!(read(&mut b).contains("<Run|"));
    clock.advance(Duration::from_secs(2));
    b.write_realtime(b'?').unwrap_or_else(|e| panic!("{e}"));
    assert!(read(&mut b).contains("<Idle|MPos:0.000,0.000"));
    assert_eq!(b.buffer_usage(), (0, 0));
    assert!(!b.laser_on());
}
#[test]
fn planner_rx_bounds_and_withheld_ack() {
    let clock = Arc::new(VirtualClock::new());
    let mut b = FakeGrbl::new(clock.clone());
    read(&mut b);
    for _ in 0..15 {
        write(&mut b, "G91G1X1F60\n");
    }
    assert_eq!(read(&mut b).matches("ok\r\n").count(), 15);
    assert_eq!(b.buffer_usage(), (15, 0));
    write(&mut b, "G1X1\n");
    assert_eq!(b.buffer_usage(), (15, 5));
    assert_eq!(read(&mut b), "");
    b.write_realtime(b'?').unwrap_or_else(|e| panic!("{e}"));
    assert!(read(&mut b).contains("Bf:0,123"));
    clock.advance(Duration::from_millis(201));
    assert_eq!(read(&mut b), "ok\r\n");
    assert_eq!(b.buffer_usage(), (15, 0));
    assert_eq!(b.write_all(&[b'X'; 129], Duration::ZERO), Err(LinkError::Timeout));
    assert_eq!(b.buffer_usage().1, 128);
    b.write_realtime(0x18).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(b.buffer_usage(), (0, 0));
    assert!(read(&mut b).contains("Grbl"));
}
#[test]
fn hold_resume_reset_and_jog_cancel() {
    let clock = Arc::new(VirtualClock::new());
    let mut b = FakeGrbl::new(clock.clone());
    read(&mut b);
    write(&mut b, "G1X10F60\nM4S20\n");
    b.write_realtime(b'!').unwrap_or_else(|e| panic!("{e}"));
    clock.advance(Duration::from_secs(3));
    b.write_realtime(b'?').unwrap_or_else(|e| panic!("{e}"));
    assert!(read(&mut b).contains("Hold:0"));
    assert_eq!(b.position(), [0.0; 3]);
    b.write_realtime(b'~').unwrap_or_else(|e| panic!("{e}"));
    clock.advance(Duration::from_secs(3));
    read(&mut b);
    assert_eq!(b.position(), [10.0, 0.0, 0.0]);
    write(&mut b, "$J=G91X20F60\n");
    assert_eq!(b.buffer_usage().0, 1);
    b.write_realtime(0x85).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(b.buffer_usage().0, 0);
    b.write_realtime(0x18).unwrap_or_else(|e| panic!("{e}"));
    assert!(!b.laser_on());
    assert_eq!(b.buffer_usage(), (0, 0));
}
#[test]
fn fragmented_compact_commands_queries_and_alarms() {
    let mut b = FakeGrbl::new(Arc::new(VirtualClock::new()));
    read(&mut b);
    write(&mut b, "G1X1");
    assert_eq!(read(&mut b), "");
    write(&mut b, "Y2F300\n");
    assert_eq!(read(&mut b), "ok\r\n");
    write(&mut b, "$$\n$I\n$G\n$#\n");
    let response = read(&mut b);
    assert!(response.contains("$32=1\r\n"));
    assert!(response.contains("[OPT:VML,15,128]"));
    assert!(response.contains("[G59:"));
    assert!(response.contains("[GC:G1"));
    b.inject_alarm(1);
    write(&mut b, "G1X10\n");
    assert!(read(&mut b).contains("error:9"));
    assert_eq!(b.buffer_usage().0, 0);
    write(&mut b, "$X\n$H\n");
    assert_eq!(read(&mut b), "ok\r\n");
    b.write_realtime(b'?').unwrap_or_else(|e| panic!("{e}"));
    assert!(read(&mut b).contains("<Home|"));
    let mut reply = Vec::new();
    b.read_available(&mut reply, Duration::from_millis(60)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(reply, b"ok\r\n");
    assert_eq!(b.position(), [0.0; 3]);
    b.inject_error_next(22);
    write(&mut b, "G1X4\n");
    assert_eq!(read(&mut b), "error:22\r\n");
    assert_eq!(b.buffer_usage().0, 0);
    write(&mut b, "G1Xnan\n");
    assert!(read(&mut b).contains("error:"));
    assert_eq!(b.buffer_usage().0, 0);
    assert!(b.set_speedup(f64::NAN).is_err());
}
