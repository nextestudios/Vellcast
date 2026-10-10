//! Protocolo binário de mídia do Vellcast (versão 1).
//!
//! Cada mensagem WebSocket binária carrega exatamente um pacote:
//! um cabeçalho fixo de [`HEADER_LEN`] bytes (little-endian) seguido do payload.
//! O formato completo está documentado em `docs/STREAMING_PROTOCOL.md`.

use bytes::{BufMut, Bytes, BytesMut};

pub mod messages;

/// Versão atual do protocolo.
pub const VERSION: u8 = 1;
/// Tamanho do cabeçalho fixo em bytes.
pub const HEADER_LEN: usize = 28;
/// Tamanho máximo de uma mensagem (cabeçalho + payload).
pub const MAX_MESSAGE_LEN: usize = 4 * 1024 * 1024;
/// Tamanho máximo de mensagens enviadas pelo cliente espectador.
pub const MAX_CLIENT_MESSAGE_LEN: usize = 1024;
/// Tamanho máximo de uma mensagem `Auth` (o token cabe com folga).
pub const MAX_AUTH_MESSAGE_LEN: usize = 2048;

/// Tipos de mensagem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    /// Configuração do decoder de vídeo (payload JSON).
    VideoInit = 0x01,
    /// Unidade de acesso H.264 em Annex B.
    VideoFrame = 0x02,
    /// Configuração do decoder de áudio (payload JSON).
    AudioInit = 0x03,
    /// Um pacote Opus.
    AudioFrame = 0x04,
    /// Cliente → servidor: medição de RTT e relógio.
    Ping = 0x10,
    /// Servidor → cliente: resposta ao `Ping`, com o mesmo payload.
    Pong = 0x11,
    /// Cliente → servidor: pede um keyframe.
    KeyframeRequest = 0x12,
    /// Transmissão encerrada.
    StreamEnd = 0x20,
    /// Servidor → espectador: o transmissor caiu; a sala aguarda a reconexão.
    StreamPaused = 0x21,
    /// Cliente → servidor: primeira mensagem da conexão (payload JSON [`messages::AuthRequest`]).
    Auth = 0x30,
    /// Servidor → cliente: autenticação aceita (payload JSON [`messages::AuthOk`]).
    AuthOk = 0x31,
    /// Servidor → cliente: erro (payload JSON [`messages::ErrorBody`]).
    Error = 0x32,
    /// Servidor → cliente: participantes ativos da sala (payload JSON [`messages::Presence`]).
    /// Clientes que não conhecem o tipo o ignoram.
    Presence = 0x40,
}

impl TryFrom<u8> for MessageType {
    type Error = DecodeError;

    fn try_from(value: u8) -> Result<Self, DecodeError> {
        Ok(match value {
            0x01 => Self::VideoInit,
            0x02 => Self::VideoFrame,
            0x03 => Self::AudioInit,
            0x04 => Self::AudioFrame,
            0x10 => Self::Ping,
            0x11 => Self::Pong,
            0x12 => Self::KeyframeRequest,
            0x20 => Self::StreamEnd,
            0x21 => Self::StreamPaused,
            0x30 => Self::Auth,
            0x31 => Self::AuthOk,
            0x32 => Self::Error,
            0x40 => Self::Presence,
            other => return Err(DecodeError::UnknownType(other)),
        })
    }
}

/// Flags do cabeçalho.
pub mod flags {
    /// O quadro é um ponto de acesso aleatório (IDR com SPS/PPS).
    pub const KEYFRAME: u16 = 1 << 0;
    /// Descontinuidade na linha do tempo (loop, troca de fonte, reconfiguração).
    pub const DISCONTINUITY: u16 = 1 << 1;
}

/// Cabeçalho fixo de todas as mensagens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub version: u8,
    pub kind: MessageType,
    pub flags: u16,
    pub stream_id: u32,
    pub sequence: u32,
    /// Timestamp de mídia em microssegundos.
    pub pts_us: u64,
    /// Relógio de parede do emissor (Unix, microssegundos).
    pub sent_at_us: u64,
}

impl Header {
    pub fn new(kind: MessageType, stream_id: u32) -> Self {
        Self {
            version: VERSION,
            kind,
            flags: 0,
            stream_id,
            sequence: 0,
            pts_us: 0,
            sent_at_us: 0,
        }
    }

    pub fn is_keyframe(&self) -> bool {
        self.flags & flags::KEYFRAME != 0
    }

    fn write(&self, buf: &mut BytesMut) {
        buf.put_u8(self.version);
        buf.put_u8(self.kind as u8);
        buf.put_u16_le(self.flags);
        buf.put_u32_le(self.stream_id);
        buf.put_u32_le(self.sequence);
        buf.put_u64_le(self.pts_us);
        buf.put_u64_le(self.sent_at_us);
    }

    /// Lê o cabeçalho sem copiar o payload.
    pub fn read(data: &[u8]) -> Result<Self, DecodeError> {
        let h: &[u8; HEADER_LEN] = data
            .get(..HEADER_LEN)
            .and_then(|s| s.try_into().ok())
            .ok_or(DecodeError::Truncated(data.len()))?;
        if h[0] != VERSION {
            return Err(DecodeError::UnsupportedVersion(h[0]));
        }
        let u16_at = |i: usize| u16::from_le_bytes([h[i], h[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]);
        let u64_at = |i: usize| {
            let mut b = [0u8; 8];
            b.copy_from_slice(&h[i..i + 8]);
            u64::from_le_bytes(b)
        };
        Ok(Self {
            version: h[0],
            kind: MessageType::try_from(h[1])?,
            flags: u16_at(2),
            stream_id: u32_at(4),
            sequence: u32_at(8),
            pts_us: u64_at(12),
            sent_at_us: u64_at(20),
        })
    }
}

/// Serializa cabeçalho + payload em um único buffer.
pub fn encode(header: &Header, payload: &[u8]) -> Result<Bytes, EncodeError> {
    let len = HEADER_LEN + payload.len();
    if len > MAX_MESSAGE_LEN {
        return Err(EncodeError::TooLarge(len));
    }
    let mut buf = BytesMut::with_capacity(len);
    header.write(&mut buf);
    buf.put_slice(payload);
    Ok(buf.freeze())
}

/// Decodifica uma mensagem, devolvendo o cabeçalho e o payload sem cópia.
pub fn decode(message: &Bytes, max_len: usize) -> Result<(Header, Bytes), DecodeError> {
    if message.len() > max_len {
        return Err(DecodeError::TooLarge(message.len()));
    }
    let header = Header::read(message)?;
    Ok((header, message.slice(HEADER_LEN..)))
}

/// Reescreve `sent_at_us` em uma mensagem já serializada.
///
/// Permite que o relay marque o instante real de envio sem
/// reconstruir a mensagem para cada espectador.
pub fn with_sent_at(message: &Bytes, sent_at_us: u64) -> Result<Bytes, DecodeError> {
    Header::read(message)?;
    let mut buf = BytesMut::from(&message[..]);
    buf[20..28].copy_from_slice(&sent_at_us.to_le_bytes());
    Ok(buf.freeze())
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EncodeError {
    #[error("mensagem de {0} bytes excede o limite")]
    TooLarge(usize),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("mensagem truncada ({0} bytes)")]
    Truncated(usize),
    #[error("mensagem de {0} bytes excede o limite")]
    TooLarge(usize),
    #[error("versão de protocolo não suportada: {0}")]
    UnsupportedVersion(u8),
    #[error("tipo de mensagem desconhecido: {0:#04x}")]
    UnknownType(u8),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Header {
        Header {
            version: VERSION,
            kind: MessageType::VideoFrame,
            flags: flags::KEYFRAME | flags::DISCONTINUITY,
            stream_id: 0xDEAD_BEEF,
            sequence: 42,
            pts_us: 16_666,
            sent_at_us: 1_700_000_000_000_000,
        }
    }

    #[test]
    fn roundtrip() {
        let msg = encode(&sample(), b"payload").expect("encode");
        assert_eq!(msg.len(), HEADER_LEN + 7);
        let (h, p) = decode(&msg, MAX_MESSAGE_LEN).expect("decode");
        assert_eq!(h, sample());
        assert!(h.is_keyframe());
        assert_eq!(&p[..], b"payload");
    }

    #[test]
    fn layout_is_little_endian_and_stable() {
        let msg = encode(&sample(), &[]).expect("encode");
        assert_eq!(msg[0], 1);
        assert_eq!(msg[1], 0x02);
        assert_eq!(&msg[2..4], &[0x03, 0x00]);
        assert_eq!(&msg[4..8], &[0xEF, 0xBE, 0xAD, 0xDE]);
        assert_eq!(&msg[8..12], &[42, 0, 0, 0]);
    }

    #[test]
    fn rejects_truncated() {
        let msg = Bytes::from_static(&[1, 2, 0]);
        assert_eq!(
            decode(&msg, MAX_MESSAGE_LEN),
            Err(DecodeError::Truncated(3))
        );
    }

    #[test]
    fn rejects_unknown_version_and_type() {
        let mut raw = encode(&sample(), &[]).expect("encode").to_vec();
        raw[0] = 2;
        assert_eq!(Header::read(&raw), Err(DecodeError::UnsupportedVersion(2)));
        raw[0] = 1;
        raw[1] = 0x7F;
        assert_eq!(Header::read(&raw), Err(DecodeError::UnknownType(0x7F)));
    }

    #[test]
    fn enforces_size_limits() {
        let big = vec![0u8; MAX_MESSAGE_LEN];
        assert!(matches!(
            encode(&sample(), &big),
            Err(EncodeError::TooLarge(_))
        ));
        let msg = encode(&Header::new(MessageType::Ping, 0), &[0; 64]).expect("encode");
        assert!(matches!(decode(&msg, 32), Err(DecodeError::TooLarge(92))));
    }

    #[test]
    fn all_types_roundtrip() {
        for code in [
            0x01u8, 0x02, 0x03, 0x04, 0x10, 0x11, 0x12, 0x20, 0x21, 0x30, 0x31, 0x32, 0x40,
        ] {
            let kind = MessageType::try_from(code).expect("tipo conhecido");
            assert_eq!(kind as u8, code);
        }
    }

    #[test]
    fn rewrites_sent_at() {
        let msg = encode(&sample(), b"x").expect("encode");
        let out = with_sent_at(&msg, 7).expect("rewrite");
        let (h, p) = decode(&out, MAX_MESSAGE_LEN).expect("decode");
        assert_eq!(h.sent_at_us, 7);
        assert_eq!(h.pts_us, sample().pts_us);
        assert_eq!(&p[..], b"x");
    }
}
