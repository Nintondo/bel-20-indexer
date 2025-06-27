use super::*;

#[derive(Clone)]
pub struct EventSender {
    pub server: Arc<Server>,
    pub event_tx: tokio::sync::broadcast::Sender<ServerEvent>,
    pub raw_event_tx: kanal::Receiver<RawServerEvent>,
}

impl EventSender {
    pub fn run(&self) -> anyhow::Result<()> {
        'outer: loop {
            let mut events = vec![];

            loop {
                match self.raw_event_tx.try_recv() {
                    Ok(Some(v)) => {
                        events.extend(v);
                    }
                    Ok(None) => {
                        if events.is_empty() {
                            if self.server.token.is_cancelled() {
                                break 'outer;
                            }

                            std::thread::sleep(Duration::from_millis(50));
                            continue;
                        }
                        break;
                    }
                    Err(_) => {
                        if events.is_empty() {
                            break 'outer;
                        }
                    }
                }
            }

            let keys = events.iter().flat_map(|(k, v)| [Some(k.address), v.action.address().copied()]).flatten().collect_vec();

            let addresses = self.server.load_addresses(keys)?;

            for (k, v) in events {
                self.event_tx
                    .send(ServerEvent::NewHistory(
                        AddressTokenIdEvent {
                            address: addresses.get(&k.address),
                            token: k.token.into(),
                            id: k.id,
                        },
                        HistoryValueEvent::into_event(v, &addresses),
                    ))
                    .ok();
            }
        }
        Ok(())
    }
}
