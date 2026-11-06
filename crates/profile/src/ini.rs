use std::fmt;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ini {
    sections: Vec<(String, Vec<(String, String)>)>,
}

impl Ini {
    pub fn parse(text: &str) -> Self {
        let mut ini = Ini::default();
        let mut current: Option<String> = None;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                current = Some(name.trim().to_owned());
                ini.section_mut(name.trim());
            } else if let (Some(section), Some((k, v))) = (&current, line.split_once('=')) {
                ini.set(section, k.trim(), v.trim());
            }
        }
        ini
    }

    pub fn sections(&self) -> impl Iterator<Item = &str> {
        self.sections.iter().map(|(name, _)| name.as_str())
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .iter()
            .find(|(name, _)| name == section)?
            .1
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        let entries = self.section_mut(section);
        match entries.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = value.to_owned(),
            None => entries.push((key.to_owned(), value.to_owned())),
        }
    }

    fn section_mut(&mut self, section: &str) -> &mut Vec<(String, String)> {
        let index = match self.sections.iter().position(|(name, _)| name == section) {
            Some(i) => i,
            None => {
                self.sections.push((section.to_owned(), Vec::new()));
                self.sections.len() - 1
            }
        };
        &mut self.sections[index].1
    }
}

impl fmt::Display for Ini {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, (name, entries)) in self.sections.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            writeln!(f, "[{name}]")?;
            for (k, v) in entries {
                writeln!(f, "{k}={v}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let text = "[General]\nVersion=2\n\n[Profile0]\nName=x\nPath=Profiles/x\n";
        let ini = Ini::parse(text);
        assert_eq!(ini.get("Profile0", "Path"), Some("Profiles/x"));
        assert_eq!(ini.to_string(), text);
    }
}
