use lazy_static::lazy_static;
use serenity::{
    async_trait,
    model::{
        channel::{Channel, Message},
        gateway::Ready,
        guild::PartialGuild,
    },
    prelude::*,
};
use std::collections::HashMap;
use structopt::StructOpt;
use tokio::sync::RwLock;

#[derive(Debug, StructOpt, Clone)]
#[structopt(name = "discord-irc", about = "A simple discord <-> IRC bridge")]
pub struct Opt {
    /// IRC nickname
    #[structopt(short, long, default_value = "PonasBridge", env = "NICKNAME")]
    pub nickname: String,

    /// Discord BOT token
    #[structopt(short, long, env = "DISCORD_TOKEN")]
    pub token: String,

    /// Discord server ID
    #[structopt(short, long, env = "DISCORD_GUILD")]
    pub guild: u64,
}

lazy_static! {
    static ref OPT: Opt = Opt::from_args();
}

struct Handler {
    // Server_Name -> irc_sender
    irc_channels: RwLock<HashMap<String, irc::client::Sender>>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if let Ok(Channel::Guild(channel)) = msg.channel_id.to_channel(&ctx).await {
            if channel.webhooks(&ctx).await.unwrap().len() == 0 {
                return;
            }
            if msg.is_own(&ctx).await {
                return;
            }
            if msg.author.bot {
                return;
            }

            let category_name = channel.category_id.unwrap().name(&ctx).await.unwrap();
            let channel_name = channel.name;
            if let Some(sender) = self.irc_channels.read().await.get(&category_name) {
                sender
                    .send_privmsg(
                        format!("##{}", channel_name),
                        format!("{}: {}", msg.author.name, msg.content),
                    )
                    .unwrap();
            }
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);

        let guild = PartialGuild::get(&ctx, OPT.guild)
            .await
            .expect("couldn't get given guild");

        // let webhooks = guild.webhooks(&ctx).await.unwrap();

        // HashMap<CategoryName, HashMap<ChannelName, Webhook>>
        let mut categories = HashMap::new();
        for (_id, channel) in guild.channels(&ctx).await.unwrap() {
            if channel.kind != serenity::model::channel::ChannelType::Text {
                continue;
            }
            if let Some(category_id) = channel.category_id {
                let category_name = category_id.name(&ctx).await.unwrap();

                let webhook = if channel.webhooks(&ctx).await.unwrap().len() > 0 {
                    channel.webhooks(&ctx).await.unwrap()[0].clone()
                } else {
                    channel
                        .create_webhook(&ctx, format!("irc_bridge_{}", channel.name))
                        .await
                        .unwrap()
                };

                if !categories.contains_key(&category_name) {
                    categories.insert(category_name.clone(), HashMap::new());
                }

                categories
                    .get_mut(&category_name)
                    .unwrap()
                    .insert(channel.name, webhook);
            }
        }

        for (category_name, channels) in categories {
            self.irc_channels
                .write()
                .await
                .insert(category_name.clone(), {
                    use irc::client::data::config::Config;
                    use irc::client::prelude::*;
                    use irc::client::Client;

                    let mut client = Client::from_config(Config {
                        nickname: Some(OPT.nickname.clone()),
                        alt_nicks: vec![
                            OPT.nickname.clone() + "_",
                            OPT.nickname.clone() + "__",
                            OPT.nickname.clone() + "___",
                        ],
                        server: Some(category_name.clone()),
                        channels: channels.keys().map(|c| format!("#{}", c)).collect(),
                        ..Config::default()
                    })
                    .await
                    .unwrap();

                    let irc_sender = client.sender();

                    let ctx_clone = ctx.clone();

                    tokio::spawn(async move {
                        use futures::*;

                        client.identify().unwrap();
                        let mut stream = client.stream().unwrap();

                        println!("IRC connected to {}", category_name);

                        while let Some(message) = stream.next().await.transpose().unwrap() {
                            let nickname = message.source_nickname().unwrap_or("Server").to_owned();
                            if let Command::PRIVMSG(channel, message_content) = message.command {
                                let discord_channel = &channel[1..];
                                channels[discord_channel]
                                    .execute(&ctx_clone, false, |w| {
                                        w.avatar_url("https://i.imgur.com/FxPoAVr.png");
                                        w.username(nickname);
                                        w.content(message_content);
                                        w
                                    })
                                    .await
                                    .unwrap();
                            }
                        }
                    });

                    irc_sender
                });
        }
    }
}

#[tokio::main]
async fn main() {
    let opt = Opt::from_args();

    let mut client = Client::builder(&opt.token)
        .event_handler(Handler {
            irc_channels: RwLock::new(HashMap::new()),
        })
        .await
        .expect("Err creating client");

    if let Err(why) = client.start().await {
        println!("Client error: {:?}", why);
    }
}
