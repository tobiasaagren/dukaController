/// Duka UDP binary protocol — packet construction and parsing.
///
/// Packet format (outgoing):
///   0xFD 0xFD | type (1) | device_id (len-prefixed) | password (len-prefixed)
///   | function (1) | data (n) | checksum (2, little-endian 16-bit sum)

// ── Enums ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum PacketFunction {
    Read      = 0x01,
    Write     = 0x02,
    WriteRead = 0x03,
    Increase  = 0x04,
    Decrease  = 0x05,
    Response  = 0x06,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum PacketParameter {
    OnOff             = 0x01,
    Speed             = 0x02,
    CurrentHumidity   = 0x25,
    ManualSpeed       = 0x44,
    Fan1Rpm           = 0x4A,
    FilterTimer       = 0x64,
    Search            = 0x7C,
    ResetAlarms       = 0x80,
    ReadAlarm         = 0x83,
    ReadFirmwareVersion = 0x86,
    FilterAlarm       = 0x88,
    VentilationMode   = 0xB7,
    UnitType          = 0xB9,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceMode {
    OneWay = 0x00,
    TwoWay = 0x01,
    In     = 0x02,
}

impl TryFrom<u8> for DeviceMode {
    type Error = u8;
    fn try_from(v: u8) -> Result<Self, u8> {
        match v {
            0x00 => Ok(DeviceMode::OneWay),
            0x01 => Ok(DeviceMode::TwoWay),
            0x02 => Ok(DeviceMode::In),
            other => Err(other),
        }
    }
}

// ── Packet construction ───────────────────────────────────────────────────────

const PACKET_START: [u8; 2] = [0xFD, 0xFD];
const PACKET_TYPE: u8 = 0x02;

fn build_packet(device_id: &str, password: &str, function: PacketFunction, data: &[u8]) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();

    buf.extend_from_slice(&PACKET_START);
    buf.push(PACKET_TYPE);

    // length-prefixed device_id
    buf.push(device_id.len() as u8);
    buf.extend_from_slice(device_id.as_bytes());

    // length-prefixed password
    buf.push(password.len() as u8);
    buf.extend_from_slice(password.as_bytes());

    buf.push(function as u8);
    buf.extend_from_slice(data);

    // Checksum skips the first 2 bytes (0xFD 0xFD), matching C# RequestPacket behaviour.
    let checksum: u16 = buf[2..].iter().map(|&b| b as u16).sum();
    buf.extend_from_slice(&checksum.to_le_bytes());

    buf
}

pub fn create_search_packet() -> Vec<u8> {
    // C# uses empty password for search packets.
    build_packet("DEFAULT_DEVICEID", "", PacketFunction::Read, &[PacketParameter::Search as u8])
}

/// Write a preset speed (1–3) to the device.
/// Passing 255 activates manual speed mode (controlled via `ManualSpeed`).
pub fn create_set_speed_packet(device_id: &str, password: &str, speed: u8) -> Vec<u8> {
    build_packet(device_id, password, PacketFunction::Write, &[PacketParameter::Speed as u8, speed])
}

pub fn create_status_packet(device_id: &str, password: &str) -> Vec<u8> {
    let data = [
        PacketParameter::OnOff as u8,
        PacketParameter::Speed as u8,
        PacketParameter::ManualSpeed as u8,
        PacketParameter::CurrentHumidity as u8,
        PacketParameter::VentilationMode as u8,
    ];
    build_packet(device_id, password, PacketFunction::Read, &data)
}

// ── Response parsing ──────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
pub struct DeviceStatus {
    pub device_id: String,
    pub is_on: bool,
    pub speed: u8,
    pub manual_speed: u8,
    pub humidity: u8,
    pub ventilation_mode: Option<DeviceMode>,
}

/// Validate checksum and parse a raw UDP response buffer into a [`DeviceStatus`].
pub fn parse_response(buf: &[u8], device_id: String) -> Option<DeviceStatus> {
    if buf.len() < 4 {
        return None;
    }

    // Validate checksum: sum of bytes[2..len-2] (skips 0xFD 0xFD), matching C# ResponsePacket.
    let (payload, checksum_bytes) = buf.split_at(buf.len() - 2);
    let expected: u16 = payload[2..].iter().map(|&b| b as u16).sum();
    let actual = u16::from_le_bytes([checksum_bytes[0], checksum_bytes[1]]);
    if expected != actual {
        return None;
    }

    // Parse parameter key-value pairs after the header
    // Header: 0xFD 0xFD | type | device_id (len-prefixed) | password (len-prefixed) | function
    let mut pos = 2; // skip 0xFD 0xFD
    pos += 1; // type

    if pos >= payload.len() { return None; }
    let id_len = payload[pos] as usize;
    pos += 1 + id_len; // skip length + device_id

    if pos >= payload.len() { return None; }
    let pw_len = payload[pos] as usize;
    pos += 1 + pw_len; // skip length + password

    pos += 1; // function byte

    let mut is_on = false;
    let mut speed = 0u8;
    let mut manual_speed = 0u8;
    let mut humidity = 0u8;
    let mut ventilation_mode = None;

    while pos + 1 < payload.len() {
        let param = payload[pos];
        let value = payload[pos + 1];
        pos += 2;

        match param {
            p if p == PacketParameter::OnOff as u8           => is_on = value != 0,
            p if p == PacketParameter::Speed as u8           => speed = value,
            p if p == PacketParameter::ManualSpeed as u8     => manual_speed = value,
            p if p == PacketParameter::CurrentHumidity as u8 => humidity = value,
            p if p == PacketParameter::VentilationMode as u8 => {
                ventilation_mode = DeviceMode::try_from(value).ok();
            }
            _ => {}
        }
    }

    Some(DeviceStatus { device_id, is_on, speed, manual_speed, humidity, ventilation_mode })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a valid response buffer by constructing the payload then appending a correct checksum.
    fn make_response(device_id: &str, password: &str, params: &[(u8, u8)]) -> Vec<u8> {
        let mut buf = vec![0xFD, 0xFD, 0x02];
        buf.push(device_id.len() as u8);
        buf.extend_from_slice(device_id.as_bytes());
        buf.push(password.len() as u8);
        buf.extend_from_slice(password.as_bytes());
        buf.push(PacketFunction::Response as u8);
        for (param, value) in params {
            buf.push(*param);
            buf.push(*value);
        }
        let checksum: u16 = buf[2..].iter().map(|&b| b as u16).sum();
        buf.extend_from_slice(&checksum.to_le_bytes());
        buf
    }

    #[test]
    fn search_packet_starts_with_magic_bytes() {
        let packet = create_search_packet();
        assert_eq!(&packet[..2], &[0xFD, 0xFD]);
    }

    #[test]
    fn search_packet_checksum_is_valid() {
        let packet = create_search_packet();
        let (payload, cs_bytes) = packet.split_at(packet.len() - 2);
        let expected: u16 = payload[2..].iter().map(|&b| b as u16).sum();
        let actual = u16::from_le_bytes([cs_bytes[0], cs_bytes[1]]);
        assert_eq!(expected, actual);
    }

    #[test]
    fn status_packet_checksum_is_valid() {
        let packet = create_status_packet("dev-01", "1111");
        let (payload, cs_bytes) = packet.split_at(packet.len() - 2);
        let expected: u16 = payload[2..].iter().map(|&b| b as u16).sum();
        let actual = u16::from_le_bytes([cs_bytes[0], cs_bytes[1]]);
        assert_eq!(expected, actual);
    }

    #[test]
    fn parse_response_decodes_fields_correctly() {
        let buf = make_response("dev-01", "1111", &[
            (PacketParameter::OnOff as u8, 0x01),
            (PacketParameter::Speed as u8, 42),
            (PacketParameter::ManualSpeed as u8, 10),
            (PacketParameter::CurrentHumidity as u8, 55),
            (PacketParameter::VentilationMode as u8, 0x01),
        ]);
        let status = parse_response(&buf, "dev-01".to_string()).expect("should parse");
        assert!(status.is_on);
        assert_eq!(status.speed, 42);
        assert_eq!(status.manual_speed, 10);
        assert_eq!(status.humidity, 55);
        assert_eq!(status.ventilation_mode, Some(DeviceMode::TwoWay));
    }

    #[test]
    fn parse_response_rejects_bad_checksum() {
        let mut buf = make_response("dev-01", "1111", &[
            (PacketParameter::OnOff as u8, 0x01),
        ]);
        // corrupt the checksum
        let last = buf.len() - 1;
        buf[last] ^= 0xFF;
        assert!(parse_response(&buf, "dev-01".to_string()).is_none());
    }

    #[test]
    fn parse_response_handles_device_off() {
        let buf = make_response("dev-01", "1111", &[
            (PacketParameter::OnOff as u8, 0x00),
        ]);
        let status = parse_response(&buf, "dev-01".to_string()).expect("should parse");
        assert!(!status.is_on);
    }

    #[test]
    fn parse_response_returns_none_for_too_short_buf() {
        assert!(parse_response(&[0xFD, 0xFD], "dev-01".to_string()).is_none());
    }
}
