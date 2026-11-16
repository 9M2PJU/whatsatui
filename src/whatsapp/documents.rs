use super::encode::{self, EncodedText, UploadedDocument};
use crate::{app::model::*, media::outgoing};
use std::{path::PathBuf, time::Duration};
use whatsapp_rust::{Client, prelude::SendError};

#[async_trait::async_trait]
trait Transport: Sync {
    fn account(&self) -> Option<AccountId>;
    async fn upload(&self, bytes: Vec<u8>) -> Result<UploadedDocument, ()>;
    async fn send(&self, encoded: EncodedText) -> Result<(), SendError>;
}
struct Native<'a>(&'a Client);
#[async_trait::async_trait]
impl Transport for Native<'_> {
    fn account(&self) -> Option<AccountId> {
        super::native::account(self.0)
    }
    async fn upload(&self, bytes: Vec<u8>) -> Result<UploadedDocument, ()> {
        self.0
            .upload(
                bytes,
                whatsapp_rust::wacore::download::MediaType::Document,
                whatsapp_rust::upload::UploadOptions::default(),
            )
            .await
            .map(Into::into)
            .map_err(|_| ())
    }
    async fn send(&self, encoded: EncodedText) -> Result<(), SendError> {
        self.0
            .send_message_with_options(encoded.to, encoded.message, encoded.options)
            .await
            .map(|_| ())
    }
}
pub(super) async fn send(client: &Client, message: OutboundText, data_dir: PathBuf) -> SendState {
    transmit(&Native(client), message, data_dir, Duration::from_secs(60)).await
}
async fn transmit(
    transport: &impl Transport,
    message: OutboundText,
    data_dir: PathBuf,
    deadline: Duration,
) -> SendState {
    let mut sending = false;
    let result = tokio::time::timeout(deadline, async {
        if transport.account().as_ref() != Some(&message.key.account) {
            return SendState::Failed;
        }
        let Some(local) = message.draft.document.clone() else {
            return SendState::Failed;
        };
        let expected = local.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            outgoing::read_document(&local, &data_dir)
        })
        .await;
        let Ok(Ok(bytes)) = prepared else {
            return SendState::Failed;
        };
        if bytes.len() as u64 != expected.size {
            return SendState::Failed;
        }
        let Ok(uploaded) = transport.upload(bytes).await else {
            return SendState::Failed;
        };
        if transport.account().as_ref() != Some(&message.key.account)
            || uploaded.attachment.size != expected.size
            || uploaded
                .attachment
                .sha256
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
                != expected.id
        {
            return SendState::Failed;
        }
        let Ok(encoded) = encode::encode_document(&message, uploaded) else {
            return SendState::Failed;
        };
        sending = true;
        encode::classify_transport_result(&transport.send(encoded).await)
    })
    .await;
    result.unwrap_or(if sending {
        SendState::Unconfirmed
    } else {
        SendState::Failed
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };
    struct Fake {
        sent: Mutex<Vec<EncodedText>>,
        fail: bool,
        switched: AtomicBool,
    }
    #[async_trait::async_trait]
    impl Transport for Fake {
        fn account(&self) -> Option<AccountId> {
            Some(
                if self.switched.load(Ordering::SeqCst) {
                    "other"
                } else {
                    "self@s.whatsapp.net"
                }
                .into(),
            )
        }
        async fn upload(&self, bytes: Vec<u8>) -> Result<UploadedDocument, ()> {
            use sha2::{Digest, Sha256};
            if self.fail {
                return Err(());
            }
            Ok(UploadedDocument {
                url: "https://mmg.whatsapp.net/v/doc".into(),
                media_key_timestamp: 7,
                attachment: crate::media::Attachment {
                    audio: None,
                    kind: crate::media::AttachmentKind::Document,
                    filename: None,
                    caption: None,
                    mime: None,
                    size: bytes.len() as u64,
                    direct_path: "/v/doc".into(),
                    media_key: [1; 32],
                    sha256: Sha256::digest(&bytes).into(),
                    encrypted_sha256: [2; 32],
                },
            })
        }
        async fn send(&self, encoded: EncodedText) -> Result<(), SendError> {
            self.sent.lock().unwrap().push(encoded);
            Ok(())
        }
    }
    #[tokio::test]
    async fn document_upload_sends_document_stanza_with_filename_and_caption() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("app.apk");
        std::fs::write(&source, b"fake-apk-bytes").unwrap();
        let local = outgoing::import_document(&source, dir.path()).unwrap();
        assert_eq!(local.mime, "application/vnd.android.package-archive");
        let message = OutboundText {
            key: MessageKey {
                account: "self@s.whatsapp.net".into(),
                chat: "551100000002@s.whatsapp.net".into(),
                sender: "self@s.whatsapp.net".into(),
                id: "doc-id".into(),
                from_me: true,
            },
            draft: Draft {
                text: "see attached".into(),
                document: Some(Box::new(local)),
                ..Default::default()
            },
            created_at_ms: 0,
        };
        let fake = Fake {
            sent: Mutex::new(vec![]),
            fail: false,
            switched: AtomicBool::new(false),
        };
        assert_eq!(
            transmit(
                &fake,
                message,
                dir.path().to_owned(),
                Duration::from_secs(2)
            )
            .await,
            SendState::Sending
        );
        let sent = fake.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        let doc = sent[0].message.document_message.as_option().unwrap();
        assert_eq!(doc.file_name.as_deref(), Some("app.apk"));
        assert_eq!(
            doc.mimetype.as_deref(),
            Some("application/vnd.android.package-archive")
        );
        assert_eq!(doc.caption.as_deref(), Some("see attached"));
        assert_eq!(sent[0].options.message_id.as_deref(), Some("doc-id"));
    }
}
