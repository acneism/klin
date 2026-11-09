use klin_shared::Url;

pub const DEFAULT_PLAYER: &str = "mpv";

fn host_is(url: &Url, domain: &str) -> bool {
    url.host_str()
        .is_some_and(|h| h == domain || h.ends_with(&format!(".{domain}")))
}

pub fn streaming_video(url: &Url) -> Option<Url> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let path = url.path();
    let youtube_watch = host_is(url, "youtube.com")
        && ((path == "/watch" && url.query_pairs().any(|(k, v)| k == "v" && !v.is_empty()))
            || path.starts_with("/shorts/")
            || path.starts_with("/live/")
            || path.starts_with("/embed/"));
    let youtu_be = url.host_str() == Some("youtu.be") && path.len() > 1;
    let twitch = host_is(url, "twitch.tv")
        && path
            .trim_matches('/')
            .split('/')
            .filter(|p| !p.is_empty())
            .count()
            >= 1
        && !matches!(
            path.trim_matches('/').split('/').next(),
            Some(
                "directory" | "downloads" | "jobs" | "p" | "settings" | "subscriptions" | "search"
            )
        );
    let rutube = host_is(url, "rutube.ru") && path.starts_with("/video/");
    let vk = (host_is(url, "vkvideo.ru") || host_is(url, "vk.com")) && path.contains("video");
    (youtube_watch || youtu_be || twitch || rutube || vk).then(|| url.clone())
}

pub fn arguments(player: &str, url: &Url) -> Vec<String> {
    let file = player.rsplit(['/', '\\']).next().unwrap_or(player);
    let name = file.to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    let mut args = Vec::new();
    if name == "mpv" {
        args.push("--force-window=immediate".to_owned());
        args.push("--keep-open=yes".to_owned());
    }
    args.push(url.to_string());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn recognizes_streaming_video_pages() {
        for page in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://m.youtube.com/watch?v=abc&t=10",
            "https://youtube.com/shorts/abc",
            "https://youtu.be/abc",
            "https://www.twitch.tv/somechannel",
            "https://rutube.ru/video/0123/",
            "https://vkvideo.ru/video-1_2",
        ] {
            assert!(streaming_video(&u(page)).is_some(), "{page}");
        }
        for page in [
            "https://www.youtube.com/",
            "https://www.youtube.com/watch",
            "https://www.youtube.com/results?search_query=x",
            "https://youtu.be/",
            "https://www.twitch.tv/directory",
            "https://notyoutube.com/watch?v=1",
            "file:///C:/video.mp4",
        ] {
            assert!(streaming_video(&u(page)).is_none(), "{page}");
        }
    }

    #[test]
    fn builds_player_arguments() {
        let url = u("https://youtu.be/abc");
        assert_eq!(
            arguments("C:\\Program Files\\mpv\\MPV.EXE", &url),
            [
                "--force-window=immediate",
                "--keep-open=yes",
                "https://youtu.be/abc"
            ]
        );
        assert_eq!(arguments("/usr/bin/vlc", &url), ["https://youtu.be/abc"]);
    }
}
