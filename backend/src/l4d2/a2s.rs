use serde::Serialize;
use std::{net::SocketAddr, time::Duration};
use tokio::net::UdpSocket;

#[derive(Debug, Serialize)]
pub struct Player {
    pub name: String,
    pub score: i32,
    pub seconds: f32,
}

#[derive(Debug, Serialize)]
pub struct Server {
    pub name: String,
    pub map: String,
    pub online: u8,
    pub capacity: u8,
    pub players: Vec<Player>,
}

struct Packet<'a>(&'a [u8]);
impl<'a> Packet<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        if self.0.len() < count {
            return Err("Respuesta A2S truncada.".into());
        }
        let (part, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(part)
    }
    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn text(&mut self) -> Result<String, String> {
        let end = self
            .0
            .iter()
            .position(|b| *b == 0)
            .ok_or("Texto A2S sin terminador.")?;
        let bytes = self.take(end + 1)?;
        let text = std::str::from_utf8(&bytes[..end]).map_err(|_| "Nombre A2S no UTF-8.")?;
        if text.chars().count() > 256 {
            return Err("Texto A2S demasiado largo.".into());
        }
        Ok(text.to_owned())
    }
}

fn payload(bytes: &[u8], kind: u8) -> Result<Packet<'_>, String> {
    if bytes.starts_with(&[0xfe, 0xff, 0xff, 0xff]) {
        return Err(
            "El servidor devuelve paquetes A2S fragmentados; esta versión no los admite.".into(),
        );
    }
    if !bytes.starts_with(&[0xff, 0xff, 0xff, 0xff, kind]) {
        return Err("Respuesta A2S inesperada o consulta bloqueada.".into());
    }
    Ok(Packet(&bytes[5..]))
}

fn info(bytes: &[u8]) -> Result<Server, String> {
    let mut p = payload(bytes, 0x49)?;
    p.byte()?;
    let name = p.text()?;
    let map = p.text()?;
    p.text()?;
    p.text()?;
    let app = p.take(2)?;
    if u16::from_le_bytes([app[0], app[1]]) != 550 {
        return Err("El servidor consultado no es Left 4 Dead 2 (550).".into());
    }
    let online = p.byte()?;
    let capacity = p.byte()?;
    if online > capacity {
        return Err("Cantidad de jugadores A2S inválida.".into());
    }
    Ok(Server {
        name,
        map,
        online,
        capacity,
        players: Vec::new(),
    })
}

fn players(bytes: &[u8]) -> Result<Vec<Player>, String> {
    let mut p = payload(bytes, 0x44)?;
    let count = p.byte()?;
    let mut result = Vec::new();
    for _ in 0..count {
        p.byte()?;
        let name = p.text()?;
        let score = p.take(4)?;
        let score = i32::from_le_bytes([score[0], score[1], score[2], score[3]]);
        let time = p.take(4)?;
        let seconds = f32::from_le_bytes([time[0], time[1], time[2], time[3]]);
        if !seconds.is_finite() || seconds < 0.0 {
            return Err("Tiempo de conexión A2S inválido.".into());
        }
        result.push(Player {
            name,
            score,
            seconds,
        });
    }
    Ok(result)
}

async fn exchange(socket: &UdpSocket, request: &[u8]) -> Result<Vec<u8>, String> {
    let base = if request[4] == 0x55 {
        &request[..5]
    } else {
        request
    };
    let mut request = request.to_vec();
    for _ in 0..3 {
        socket
            .send(&request)
            .await
            .map_err(|_| "No se pudo enviar la consulta A2S.")?;
        let mut buffer = vec![0; 65507];
        let length = socket
            .recv(&mut buffer)
            .await
            .map_err(|_| "No se pudo recibir la respuesta A2S.")?;
        buffer.truncate(length);
        if buffer.starts_with(&[0xff, 0xff, 0xff, 0xff, 0x41]) {
            if buffer.len() != 9 {
                return Err("Desafío A2S inválido.".into());
            }
            request = base.to_vec();
            request.extend_from_slice(&buffer[5..9]);
        } else {
            return Ok(buffer);
        }
    }
    Err("Demasiados desafíos A2S.".into())
}

pub async fn query(address: SocketAddr) -> Result<Server, String> {
    tokio::time::timeout(Duration::from_secs(6), async {
        let bind = if address.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let socket = UdpSocket::bind(bind)
            .await
            .map_err(|_| "No se pudo abrir UDP para A2S.")?;
        socket
            .connect(address)
            .await
            .map_err(|_| "No se pudo conectar al puerto A2S.")?;
        let mut result =
            info(&exchange(&socket, b"\xff\xff\xff\xff\x54Source Engine Query\0").await?)?;
        result.players =
            players(&exchange(&socket, b"\xff\xff\xff\xff\x55\xff\xff\xff\xff").await?)?;
        Ok(result)
    })
    .await
    .map_err(|_| {
        "El servidor no respondió a A2S en 6 segundos. Revisa IP, puerto de consulta y firewall."
            .to_string()
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_names_scores_and_times_without_claiming_steam_identity() {
        let mut data = b"\xff\xff\xff\xff\x44\x01\x00Alice\0".to_vec();
        data.extend_from_slice(&(-4i32).to_le_bytes());
        data.extend_from_slice(&120f32.to_le_bytes());
        let result = players(&data).unwrap();
        assert_eq!(result[0].name, "Alice");
        assert_eq!(result[0].score, -4);
        assert_eq!(result[0].seconds, 120.0);
        data.truncate(data.len() - 1);
        assert!(players(&data).is_err());
    }
    #[test]
    fn rejects_fragmented_wrong_game_and_invalid_times() {
        assert!(players(b"\xfe\xff\xff\xff")
            .unwrap_err()
            .contains("fragmentados"));
        let mut data = b"\xff\xff\xff\xff\x49\x11Server\0map\0folder\0Game\0".to_vec();
        data.extend_from_slice(&440u16.to_le_bytes());
        data.extend_from_slice(&[1, 4]);
        assert!(info(&data).unwrap_err().contains("550"));
        let mut data = b"\xff\xff\xff\xff\x44\x01\x00Alice\0".to_vec();
        data.extend_from_slice(&0i32.to_le_bytes());
        data.extend_from_slice(&f32::NAN.to_le_bytes());
        assert!(players(&data).is_err());
    }

    #[tokio::test]
    async fn udp_query_handles_info_and_player_challenges_and_checks_game() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut buffer = [0; 1024];
            let (length, peer) = socket.recv_from(&mut buffer).await.unwrap();
            assert_eq!(
                &buffer[..length],
                b"\xff\xff\xff\xff\x54Source Engine Query\0"
            );
            socket
                .send_to(b"\xff\xff\xff\xff\x41\x01\x02\x03\x04", peer)
                .await
                .unwrap();
            let (length, _) = socket.recv_from(&mut buffer).await.unwrap();
            assert!(buffer[..length].ends_with(&[1, 2, 3, 4]));
            let mut response =
                b"\xff\xff\xff\xff\x49\x11Server\0c1m1_hotel\0left4dead2\0Left 4 Dead 2\0".to_vec();
            response.extend_from_slice(&550u16.to_le_bytes());
            response.extend_from_slice(&[1, 4]);
            socket.send_to(&response, peer).await.unwrap();
            let (length, _) = socket.recv_from(&mut buffer).await.unwrap();
            assert_eq!(&buffer[..length], b"\xff\xff\xff\xff\x55\xff\xff\xff\xff");
            socket
                .send_to(b"\xff\xff\xff\xff\x41\x05\x06\x07\x08", peer)
                .await
                .unwrap();
            let (length, _) = socket.recv_from(&mut buffer).await.unwrap();
            assert_eq!(&buffer[..length], b"\xff\xff\xff\xff\x55\x05\x06\x07\x08");
            let mut response = b"\xff\xff\xff\xff\x44\x01\x00Alice\0".to_vec();
            response.extend_from_slice(&42i32.to_le_bytes());
            response.extend_from_slice(&90f32.to_le_bytes());
            socket.send_to(&response, peer).await.unwrap();
        });
        let result = query(address).await.unwrap();
        assert_eq!(result.map, "c1m1_hotel");
        assert_eq!(result.players[0].score, 42);
        server.await.unwrap();
    }
}
