#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    // English
    English, // en

    // Chinese
    Chinese,            // zh
    ChineseSimplified,  // zh-CN
    ChineseTraditional, // zh-TW

    // Japanese
    Japanese, // ja

    // Korean
    Korean, // ko

    // European languages
    French,     // fr
    German,     // de
    Spanish,    // es
    Portuguese, // pt
    Italian,    // it
    Russian,    // ru
    Dutch,      // nl
    Polish,     // pl
    Turkish,    // tr

    // South & Southeast Asia
    Hindi,      // hi
    Thai,       // th
    Vietnamese, // vi
    Indonesian, // id

    // Middle East
    Arabic,  // ar
    Hebrew,  // he
    Persian, // fa

    // Nordic
    Swedish,   // sv
    Danish,    // da
    Norwegian, // no
    Finnish,   // fi

    // Others
    Greek,     // el
    Czech,     // cs
    Hungarian, // hu
    Romanian,  // ro
    Ukrainian, // uk
}

impl Language {
    pub fn code(&self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Chinese => "zh",
            Language::ChineseSimplified => "zh-CN",
            Language::ChineseTraditional => "zh-TW",
            Language::Japanese => "ja",
            Language::Korean => "ko",
            Language::French => "fr",
            Language::German => "de",
            Language::Spanish => "es",
            Language::Portuguese => "pt",
            Language::Italian => "it",
            Language::Russian => "ru",
            Language::Dutch => "nl",
            Language::Polish => "pl",
            Language::Turkish => "tr",
            Language::Hindi => "hi",
            Language::Thai => "th",
            Language::Vietnamese => "vi",
            Language::Indonesian => "id",
            Language::Arabic => "ar",
            Language::Hebrew => "he",
            Language::Persian => "fa",
            Language::Swedish => "sv",
            Language::Danish => "da",
            Language::Norwegian => "no",
            Language::Finnish => "fi",
            Language::Greek => "el",
            Language::Czech => "cs",
            Language::Hungarian => "hu",
            Language::Romanian => "ro",
            Language::Ukrainian => "uk",
        }
    }
}

pub const SUPPORTED_LANGUAGES: &[Language] = &[
    Language::English,
    Language::Chinese,
    Language::ChineseSimplified,
    Language::ChineseTraditional,
    Language::Japanese,
    Language::Korean,
    Language::French,
    Language::German,
    Language::Spanish,
    Language::Portuguese,
    Language::Italian,
    Language::Russian,
    Language::Dutch,
    Language::Polish,
    Language::Turkish,
    Language::Hindi,
    Language::Thai,
    Language::Vietnamese,
    Language::Indonesian,
    Language::Arabic,
    Language::Hebrew,
    Language::Persian,
    Language::Swedish,
    Language::Danish,
    Language::Norwegian,
    Language::Finnish,
    Language::Greek,
    Language::Czech,
    Language::Hungarian,
    Language::Romanian,
    Language::Ukrainian,
];
