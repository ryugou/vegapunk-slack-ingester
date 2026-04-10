use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct SlackApiResponse<T> {
    pub ok: bool,
    pub error: Option<String>,
    #[serde(flatten)]
    pub data: Option<T>,
}

#[derive(Debug, Deserialize)]
pub struct UserInfoData {
    pub user: SlackUser,
}

#[derive(Debug, Deserialize)]
pub struct SlackUser {
    pub id: String,
    pub real_name: Option<String>,
    pub profile: SlackUserProfile,
}

#[derive(Debug, Deserialize)]
pub struct SlackUserProfile {
    pub display_name: Option<String>,
    pub real_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ConversationInfoData {
    pub channel: SlackChannel,
}

#[derive(Debug, Deserialize)]
pub struct SlackChannel {
    pub id: String,
    pub name: Option<String>,
    pub purpose: Option<SlackPurpose>,
}

#[derive(Debug, Deserialize)]
pub struct SlackPurpose {
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct HistoryData {
    pub messages: Vec<HistoryMessage>,
    pub has_more: Option<bool>,
    pub response_metadata: Option<ResponseMetadata>,
}

#[derive(Debug, Deserialize)]
pub struct HistoryMessage {
    pub r#type: Option<String>,
    pub user: Option<String>,
    pub text: Option<String>,
    pub ts: String,
    pub thread_ts: Option<String>,
    pub bot_id: Option<String>,
    pub reply_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseMetadata {
    pub next_cursor: Option<String>,
}
