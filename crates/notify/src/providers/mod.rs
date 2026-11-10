//! One module per channel kind, ported from Uptime Kuma's notification
//! providers. Each declares its settings next to the code that uses them.

mod alerta;
mod alertnow;
mod aliyun_sms;
mod amootsms;
mod apprise;
mod bale;
mod bark;
mod bearsms;
mod bitrix24;
mod brevo;
mod call_me_bot;
mod cellsynt;
mod clicksend_sms;
mod clickup;
mod dingding;
mod discord;
mod egosms;
mod elks_46;
mod evolution;
mod feishu;
mod flashduty;
mod flowtriq;
mod fluxer;
mod freemobile;
mod goalert;
mod google_chat;
mod google_sheets;
mod gorush;
mod gotify;
mod grafana_oncall;
mod gtx_messaging;
mod halopsa;
mod heii_oncall;
mod home_assistant;
mod indigo;
mod jira_service_management;
mod keep;
mod kook;
mod line;
mod lunasea;
mod matrix;
mod mattermost;
mod max;
mod messenger_360;
mod microsoft_graph;
mod milky;
mod nextcloud_talk;
mod notifery;
mod notifyapp;
mod ntfy;
mod octopush;
mod onebot;
mod onechat;
mod onesender;
mod ooredoo;
mod openwa;
mod opsgenie;
mod opsimate;
mod pagerduty;
mod pagertree;
mod pinglet;
mod plivo;
mod promosms;
mod pumble;
mod pushbullet;
mod pushdeer;
mod pushover;
mod pushplus;
mod pushy;
mod resend;
mod rocket_chat;
mod sendgrid;
mod serverchan;
mod serwersms;
mod sevenio;
mod signal;
mod signalgrid;
mod signl4;
mod slack;
mod sms_gateway;
mod sms_planet;
mod smsc;
mod smseagle;
mod smsir;
mod smsmanager;
mod smspartner;
mod smtp;
mod splunk;
mod spugpush;
mod squadcast;
mod stackfield;
mod teams;
mod techulus_push;
mod telegram;
mod telnyx;
mod teltonika;
mod threema;
mod turbosmtp;
mod twilio;
mod util;
mod vk;
mod vkteams;
mod waha;
mod webhook;
mod wecom;
mod whapi;
mod wpush;
mod wxpusher;
mod yzj;
mod zoho_cliq;

pub(crate) mod prelude {
    pub use super::util::*;
    pub use crate::{
        body_code, by_status, Field, HttpRequest, MailRequest, MailSecurity, Message, Provider,
        Request, Response, Settings, Status, Verdict,
    };
    pub use serde_json::{json, Value};
}

/// Sorted by kind.
pub(crate) static ALL: &[&crate::Provider] = &[
    &messenger_360::PROVIDER,
    &elks_46::PROVIDER,
    &alerta::PROVIDER,
    &alertnow::PROVIDER,
    &aliyun_sms::PROVIDER,
    &amootsms::PROVIDER,
    &apprise::PROVIDER,
    &bale::PROVIDER,
    &bark::PROVIDER,
    &bearsms::PROVIDER,
    &bitrix24::PROVIDER,
    &brevo::PROVIDER,
    &call_me_bot::PROVIDER,
    &cellsynt::PROVIDER,
    &clicksend_sms::PROVIDER,
    &clickup::PROVIDER,
    &dingding::PROVIDER,
    &discord::PROVIDER,
    &egosms::PROVIDER,
    &evolution::PROVIDER,
    &feishu::PROVIDER,
    &flashduty::PROVIDER,
    &flowtriq::PROVIDER,
    &fluxer::PROVIDER,
    &freemobile::PROVIDER,
    &goalert::PROVIDER,
    &google_chat::PROVIDER,
    &google_sheets::PROVIDER,
    &gorush::PROVIDER,
    &gotify::PROVIDER,
    &grafana_oncall::PROVIDER,
    &gtx_messaging::PROVIDER,
    &halopsa::PROVIDER,
    &heii_oncall::PROVIDER,
    &home_assistant::PROVIDER,
    &indigo::PROVIDER,
    &jira_service_management::PROVIDER,
    &keep::PROVIDER,
    &kook::PROVIDER,
    &line::PROVIDER,
    &lunasea::PROVIDER,
    &matrix::PROVIDER,
    &mattermost::PROVIDER,
    &max::PROVIDER,
    &microsoft_graph::PROVIDER,
    &milky::PROVIDER,
    &nextcloud_talk::PROVIDER,
    &notifery::PROVIDER,
    &notifyapp::PROVIDER,
    &ntfy::PROVIDER,
    &octopush::PROVIDER,
    &onebot::PROVIDER,
    &onechat::PROVIDER,
    &onesender::PROVIDER,
    &ooredoo::PROVIDER,
    &openwa::PROVIDER,
    &opsgenie::PROVIDER,
    &opsimate::PROVIDER,
    &pagerduty::PROVIDER,
    &pagertree::PROVIDER,
    &pinglet::PROVIDER,
    &plivo::PROVIDER,
    &promosms::PROVIDER,
    &pumble::PROVIDER,
    &pushbullet::PROVIDER,
    &pushdeer::PROVIDER,
    &pushover::PROVIDER,
    &pushplus::PROVIDER,
    &pushy::PROVIDER,
    &resend::PROVIDER,
    &rocket_chat::PROVIDER,
    &sendgrid::PROVIDER,
    &serverchan::PROVIDER,
    &serwersms::PROVIDER,
    &sevenio::PROVIDER,
    &signal::PROVIDER,
    &signalgrid::PROVIDER,
    &signl4::PROVIDER,
    &slack::PROVIDER,
    &sms_gateway::PROVIDER,
    &sms_planet::PROVIDER,
    &smsc::PROVIDER,
    &smseagle::PROVIDER,
    &smsir::PROVIDER,
    &smsmanager::PROVIDER,
    &smspartner::PROVIDER,
    &smtp::PROVIDER,
    &splunk::PROVIDER,
    &spugpush::PROVIDER,
    &squadcast::PROVIDER,
    &stackfield::PROVIDER,
    &teams::PROVIDER,
    &techulus_push::PROVIDER,
    &telegram::PROVIDER,
    &telnyx::PROVIDER,
    &teltonika::PROVIDER,
    &threema::PROVIDER,
    &turbosmtp::PROVIDER,
    &twilio::PROVIDER,
    &vk::PROVIDER,
    &vkteams::PROVIDER,
    &waha::PROVIDER,
    &webhook::PROVIDER,
    &wecom::PROVIDER,
    &whapi::PROVIDER,
    &wpush::PROVIDER,
    &wxpusher::PROVIDER,
    &yzj::PROVIDER,
    &zoho_cliq::PROVIDER,
];

#[cfg(test)]
mod tests;
