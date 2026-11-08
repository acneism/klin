#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BlockCategory {
    Social,
    Tracking,
    Cryptomining,
    Fingerprinting,
}

impl BlockCategory {
    pub fn label(self) -> &'static str {
        match self {
            BlockCategory::Social => "Трекеры социальных сетей",
            BlockCategory::Tracking => "Отслеживающее содержимое",
            BlockCategory::Cryptomining => "Криптомайнеры",
            BlockCategory::Fingerprinting => "Сборщики цифровых отпечатков",
        }
    }
}
