use crate::adapters::sqlite::knowledge_repo::KnowledgeRepository;
use crate::config::AppConfig;
use crate::error::{HydrogenError, Result};
use clap::{Args, Subcommand};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::json;
use std::process::Command;

#[derive(Args, Debug)]
pub struct WebArgs {
    #[command(subcommand)]
    pub command: WebCommands,
}

#[derive(Subcommand, Debug)]
pub enum WebCommands {
    /// Add an RSS or Atom feed URL
    FeedAdd { url: String },
    /// List configured feeds
    FeedList,
    /// Fetch feeds and index their readable article content
    FeedSync,
}

#[derive(Debug, Default)]
struct FeedItem {
    title: String,
    link: String,
    content: String,
    published: String,
    author: String,
}

fn local_name(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_ascii_lowercase()
}

fn parse_feed(xml: &str) -> (String, Vec<FeedItem>) {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut feed_title = String::new();
    let mut current = FeedItem::default();
    let mut current_tag = String::new();
    let mut in_item = false;
    let mut items = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                let tag = local_name(event.name().as_ref());
                if tag == "item" || tag == "entry" {
                    current = FeedItem::default();
                    in_item = true;
                } else if in_item {
                    current_tag = tag;
                    if current_tag == "link" {
                        for attribute in event.attributes().flatten() {
                            if attribute.key.as_ref() == b"href" {
                                if let Ok(value) = attribute.unescape_value() {
                                    current.link = value.into_owned();
                                }
                            }
                        }
                    }
                } else if tag == "title" {
                    current_tag = "feed-title".into();
                }
            }
            Ok(Event::Empty(event)) => {
                if in_item && local_name(event.name().as_ref()) == "link" {
                    for attribute in event.attributes().flatten() {
                        if attribute.key.as_ref() == b"href" {
                            if let Ok(value) = attribute.unescape_value() {
                                current.link = value.into_owned();
                            }
                        }
                    }
                }
            }
            Ok(Event::Text(event)) => {
                if let Ok(value) = event.unescape() {
                    let value = value.trim();
                    if in_item {
                        match current_tag.as_str() {
                            "title" => current.title.push_str(value),
                            "link" if current.link.is_empty() => current.link.push_str(value),
                            "description" | "summary" | "content" | "content:encoded" | "encoded" => {
                                current.content.push_str(value);
                            }
                            "pubdate" | "published" | "updated" | "dc:date" => {
                                current.published.push_str(value);
                            }
                            "author" | "dc:creator" | "name" => {
                                if current.author.is_empty() {
                                    current.author.push_str(value);
                                }
                            }
                            _ => {}
                        }
                    } else if current_tag == "feed-title" {
                        feed_title.push_str(value);
                    }
                }
            }
            Ok(Event::CData(event)) => {
                if in_item {
                    current.content.push_str(&String::from_utf8_lossy(event.as_ref()));
                }
            }
            Ok(Event::End(event)) => {
                let tag = local_name(event.name().as_ref());
                if tag == "item" || tag == "entry" {
                    if !current.link.is_empty() {
                        items.push(std::mem::take(&mut current));
                    }
                    in_item = false;
                }
                current_tag.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    (feed_title, items)
}

fn readable_text(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut in_tag = false;
    for character in input.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => output.push(character),
            _ => {}
        }
    }
    output
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn feed_sync(cfg: &AppConfig) -> Result<()> {
    if !cfg.settings.web.enabled || !cfg.settings.web.feeds_enabled {
        return Err(HydrogenError::Config("web feed indexing is disabled".into()));
    }
    let repo = KnowledgeRepository::open(&cfg.db_path)?;
    let feeds = repo.list_feeds()?;
    let mut report = Vec::new();

    for feed in feeds.into_iter().filter(|feed| feed.enabled) {
        let output = Command::new("curl")
            .args([
                "--silent",
                "--show-error",
                "--location",
                "--fail",
                "--max-time",
                "25",
                "--max-filesize",
                "5242880",
                "--user-agent",
                "Hydrogen/0.1 (local RSS reader)",
                &feed.url,
            ])
            .output();

        let result = match output {
            Ok(output) if output.status.success() => {
                let xml = String::from_utf8_lossy(&output.stdout);
                let (feed_title, items) = parse_feed(&xml);
                let mut indexed = 0usize;
                for item in items {
                    let text = readable_text(&item.content);
                    if text.chars().count() < 100 {
                        continue;
                    }
                    let site = if feed_title.is_empty() {
                        feed.url.split("://").nth(1).unwrap_or(&feed.url)
                            .split('/').next().unwrap_or(&feed.url)
                    } else {
                        &feed_title
                    };
                    let hash = blake3::hash(text.as_bytes()).to_hex().to_string();
                    repo.save_web_page(
                        &item.link,
                        &item.link,
                        &item.title,
                        &item.author,
                        &item.published,
                        site,
                        &text,
                        "",
                        &hash,
                        false,
                        false,
                        cfg.settings.web.version_pages,
                    )?;
                    indexed += 1;
                }
                repo.mark_feed_checked(feed.id, &feed_title, None)?;
                json!({"url":feed.url,"indexed":indexed,"status":"ok"})
            }
            Ok(output) => {
                let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
                repo.mark_feed_checked(feed.id, "", Some(&error))?;
                json!({"url":feed.url,"status":"error","error":error})
            }
            Err(error) => {
                let message = format!("could not start curl: {error}");
                repo.mark_feed_checked(feed.id, "", Some(&message))?;
                json!({"url":feed.url,"status":"error","error":message})
            }
        };
        report.push(result);
    }
    println!("{}", serde_json::json!({"feeds":report}));
    Ok(())
}

pub fn run(args: WebArgs, cfg: &AppConfig) -> Result<()> {
    let repo = KnowledgeRepository::open(&cfg.db_path)?;
    match args.command {
        WebCommands::FeedAdd { url } => {
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(HydrogenError::InvalidInput("feed URL must use HTTP or HTTPS".into()));
            }
            let id = repo.add_feed(&url)?;
            println!("{}", serde_json::json!({"status":"added","id":id,"url":url}));
            Ok(())
        }
        WebCommands::FeedList => {
            println!("{}", serde_json::to_string_pretty(&repo.list_feeds()?)?);
            Ok(())
        }
        WebCommands::FeedSync => feed_sync(cfg),
    }
}
