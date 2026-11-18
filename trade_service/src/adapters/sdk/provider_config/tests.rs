use super::*;
use crate::config::{AstralaneConfig, BlockRazorConfig};

fn provider(name: &str, enabled: bool) -> ProviderConfig {
    ProviderConfig {
        enabled,
        auth_token: Some(format!("fixture-{name}")),
        region: "Frankfurt".into(),
        custom_url: Some(format!("https://{name}.example.invalid/path?fixture=1")),
    }
}

fn settings(mask: u8) -> MevConfig {
    MevConfig {
        enable_mev_protection: false,
        require_mev_submit: false,
        use_backup_rpcs: false,
        jito: provider("jito", mask & 1 != 0),
        temporal: provider("temporal", mask & 2 != 0),
        flashblock: provider("flashblock", mask & 4 != 0),
        blockrazor: BlockRazorConfig {
            provider: provider("blockrazor", mask & 8 != 0),
            transport: "grpc".into(),
        },
        astralane: AstralaneConfig {
            provider: provider("astralane", mask & 16 != 0),
            transport: "binary".into(),
        },
        speedlanding: provider("speedlanding", mask & 32 != 0),
    }
}

fn providers(mev: &mut MevConfig) -> [&mut ProviderConfig; 6] {
    [
        &mut mev.jito,
        &mut mev.temporal,
        &mut mev.flashblock,
        &mut mev.blockrazor.provider,
        &mut mev.astralane.provider,
        &mut mev.speedlanding,
    ]
}

#[test]
fn sdk_provider_config_preserves_all_enable_subsets_original_fields_and_order() {
    let region = SwqosRegion::Frankfurt;
    let key = |name| format!("fixture-{name}");
    let url = |name| Some(format!("https://{name}.example.invalid/path?fixture=1"));
    let all = [
        SwqosConfig::Jito(key("jito"), region.clone(), url("jito"), None),
        SwqosConfig::Temporal(key("temporal"), region.clone(), url("temporal"), None),
        SwqosConfig::FlashBlock(key("flashblock"), region.clone(), url("flashblock"), None),
        SwqosConfig::BlockRazor(
            key("blockrazor"),
            region.clone(),
            url("blockrazor"),
            Some(SwqosTransport::Grpc),
            None,
        ),
        SwqosConfig::Astralane(
            key("astralane"),
            region.clone(),
            url("astralane"),
            Some(AstralaneTransport::Binary),
            None,
        ),
        SwqosConfig::Speedlanding(key("speedlanding"), region, url("speedlanding"), None),
    ];
    let mut cases = 0;
    for mask in 0u8..64 {
        let expected = all
            .iter()
            .enumerate()
            .filter(|(index, _)| mask & (1 << index) != 0)
            .map(|(_, config)| config.clone())
            .collect::<Vec<_>>();
        for flags in 0u8..8 {
            let mut mev = settings(mask);
            mev.enable_mev_protection = flags & 1 != 0;
            mev.require_mev_submit = flags & 2 != 0;
            mev.use_backup_rpcs = flags & 4 != 0;
            let actual = build_swqos_configs(&mev).unwrap();
            assert_eq!(actual, expected, "mask={mask}, flags={flags}");
            assert!(!actual.iter().any(|v| v.swqos_type() == SwqosType::Default));
            cases += 1;
        }
    }
    assert_eq!(cases, 512);
}

#[test]
fn sdk_provider_config_keeps_disabled_settings_inert_and_original_error_priority() {
    let mut disabled = settings(0);
    disabled.blockrazor.transport = "unsupported-disabled-transport".into();
    disabled.astralane.transport = "unsupported-disabled-mode".into();
    for value in providers(&mut disabled) {
        value.auth_token = None;
        value.region = "unsupported-disabled-region".into();
    }
    assert!(build_swqos_configs(&disabled).unwrap().is_empty());

    let names = [
        "JITO",
        "TEMPORAL",
        "FLASHBLOCK",
        "BLOCKRAZOR",
        "ASTRALANE",
        "SPEEDLANDING",
    ];
    for index in 0..6 {
        let mut mev = settings(63);
        // This provider and every later provider are invalid: the earliest wins.
        for value in providers(&mut mev).into_iter().skip(index) {
            value.auth_token = None;
            value.region = "invalid-region".into();
        }
        let expected = if index == 0 {
            "unsupported SWQoS region: invalid-region".to_string()
        } else {
            format!(
                "{} is enabled but auth token/api key/uuid is missing",
                names[index]
            )
        };
        assert_eq!(build_swqos_configs(&mev).unwrap_err().to_string(), expected);

        let mut mev = settings(1 << index);
        providers(&mut mev)[index].region = "invalid-region".into();
        mev.blockrazor.transport = "invalid-transport".into();
        mev.astralane.transport = "invalid-mode".into();
        assert_eq!(
            build_swqos_configs(&mev).unwrap_err().to_string(),
            "unsupported SWQoS region: invalid-region"
        );
    }
}

#[test]
fn sdk_provider_config_preserves_region_aliases_and_distinct_transport_defaults() {
    for (aliases, expected) in [
        (
            &["newyork", "ny", "NYC", " New_York "][..],
            SwqosRegion::NewYork,
        ),
        (&["frankfurt", "fra"][..], SwqosRegion::Frankfurt),
        (&["amsterdam", "ams"][..], SwqosRegion::Amsterdam),
        (&["dublin", "dub"][..], SwqosRegion::Dublin),
        (
            &["slc", "saltlakecity", "Salt-Lake City"][..],
            SwqosRegion::SLC,
        ),
        (&["tokyo", "tyo"][..], SwqosRegion::Tokyo),
        (&["singapore", "sg", "sin"][..], SwqosRegion::Singapore),
        (&["london", "lon"][..], SwqosRegion::London),
        (&["losangeles", "la", "lax"][..], SwqosRegion::LosAngeles),
        (&["default", "", " _- "][..], SwqosRegion::Default),
    ] {
        for alias in aliases {
            assert_eq!(parse_region(alias).unwrap(), expected);
        }
    }
    for (alias, expected) in [
        ("", SwqosTransport::Http),
        (" HTTP ", SwqosTransport::Http),
        ("g_r-p c", SwqosTransport::Grpc),
        ("Quic", SwqosTransport::Quic),
    ] {
        let mut mev = settings(8);
        mev.blockrazor.transport = alias.into();
        assert!(matches!(
            &build_swqos_configs(&mev).unwrap()[0],
            SwqosConfig::BlockRazor(_, _, _, Some(actual), None) if *actual == expected
        ));
    }
    for (alias, expected) in [
        ("", AstralaneTransport::Binary),
        (" Binary ", AstralaneTransport::Binary),
        ("http", AstralaneTransport::Plain),
        ("plain", AstralaneTransport::Plain),
        ("Q_u-i c", AstralaneTransport::Quic),
    ] {
        let mut mev = settings(16);
        mev.astralane.transport = alias.into();
        assert!(matches!(
            &build_swqos_configs(&mev).unwrap()[0],
            SwqosConfig::Astralane(_, _, _, Some(actual), None) if *actual == expected
        ));
    }
    for (mask, expected) in [
        (8, "unsupported SWQoS transport: invalid"),
        (16, "unsupported Astralane transport: invalid"),
    ] {
        let mut mev = settings(mask);
        mev.blockrazor.transport = "invalid".into();
        mev.astralane.transport = "invalid".into();
        assert_eq!(build_swqos_configs(&mev).unwrap_err().to_string(), expected);
    }
    assert_eq!(
        parse_region("unrecognized").unwrap_err().to_string(),
        "unsupported SWQoS region: unrecognized"
    );
}

#[test]
fn sdk_provider_config_keeps_optional_jito_auth_and_exact_owned_credentials() {
    let mut mev = settings(1);
    mev.jito.auth_token = None;
    mev.jito.custom_url = None;
    assert_eq!(
        build_swqos_configs(&mev).unwrap(),
        vec![SwqosConfig::Jito(
            String::new(),
            SwqosRegion::Frankfurt,
            None,
            None
        )]
    );
    let mut mev = settings(63);
    for value in providers(&mut mev) {
        value.auth_token = Some(" fixture-token with spaces ".into());
        value.custom_url = Some(" fixture-url retained verbatim ".into());
    }
    let owned = build_swqos_configs(&mev).unwrap();
    for value in providers(&mut mev) {
        value.auth_token.as_mut().unwrap().clear();
        value.custom_url.as_mut().unwrap().clear();
    }
    assert_ne!(owned, build_swqos_configs(&mev).unwrap());
    for config in owned {
        let (token, url) = match config {
            SwqosConfig::Jito(token, _, url, _)
            | SwqosConfig::Temporal(token, _, url, _)
            | SwqosConfig::FlashBlock(token, _, url, _)
            | SwqosConfig::Speedlanding(token, _, url, _)
            | SwqosConfig::BlockRazor(token, _, url, _, _)
            | SwqosConfig::Astralane(token, _, url, _, _) => (token, url),
            _ => panic!("service config added an unreviewed provider"),
        };
        assert_eq!(token, " fixture-token with spaces ");
        assert_eq!(url.as_deref(), Some(" fixture-url retained verbatim "));
    }
    // Preserve the old distinction: missing auth is rejected; explicit empty auth is forwarded.
    assert_eq!(build_swqos_configs(&mev).unwrap().len(), 6);
}

#[test]
fn sdk_provider_config_maps_only_existing_public_choices_and_preserves_guard_policy() {
    let choices = [
        (MevProvider::Auto, None, "auto MEV"),
        (MevProvider::Default, None, "default RPC"),
        (MevProvider::Jito, Some(SwqosType::Jito), "jito"),
        (MevProvider::Temporal, Some(SwqosType::Temporal), "temporal"),
        (
            MevProvider::FlashBlock,
            Some(SwqosType::FlashBlock),
            "flashblock",
        ),
        (
            MevProvider::BlockRazor,
            Some(SwqosType::BlockRazor),
            "blockrazor",
        ),
        (
            MevProvider::Astralane,
            Some(SwqosType::Astralane),
            "astralane",
        ),
        (
            MevProvider::Speedlanding,
            Some(SwqosType::Speedlanding),
            "speedlanding",
        ),
    ];
    for (choice, expected, name) in choices {
        assert_eq!(provider_swqos_type(choice), expected);
        assert_eq!(provider_name(choice), name);
        for use_mev in [None, Some(false), Some(true)] {
            for actual in [
                SwqosType::Default,
                SwqosType::Jito,
                SwqosType::Temporal,
                SwqosType::FlashBlock,
                SwqosType::BlockRazor,
                SwqosType::Astralane,
                SwqosType::Speedlanding,
                SwqosType::Node1,
            ] {
                let allowed = match (use_mev, choice) {
                    (Some(false), MevProvider::Auto | MevProvider::Default) => {
                        actual == SwqosType::Default
                    }
                    (Some(false), _) | (Some(true), MevProvider::Default) => false,
                    (_, MevProvider::Default) => actual == SwqosType::Default,
                    (Some(true), MevProvider::Auto) => actual != SwqosType::Default,
                    (None, MevProvider::Auto) => true,
                    _ => expected == Some(actual),
                };
                let result = super::super::ordinary_lanes::ensure_requested_provider(
                    use_mev,
                    Some(choice),
                    actual,
                );
                assert_eq!(result.is_ok(), allowed);
                if let Err(error) = result {
                    assert_eq!(
                        error.to_string(),
                        "DAMM selected provider differs from the original MEV policy"
                    );
                }
            }
        }
    }
}
