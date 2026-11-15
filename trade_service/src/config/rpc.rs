const BACKUP_KEYS: [&str; 3] = ["BACKUP_RPC_URL", "BACKUP_RPC_URLS", "RPC_BACKUP_URLS"];

pub(super) fn from_env() -> anyhow::Result<Vec<String>> {
    let primary = super::required("RPC_URL")?;
    Ok(merge_urls(
        primary,
        BACKUP_KEYS.map(|key| std::env::var(key).ok()),
    ))
}

fn merge_urls(primary: String, backups: [Option<String>; 3]) -> Vec<String> {
    let mut urls = vec![primary];
    for value in backups.into_iter().flatten() {
        urls.extend(
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
        );
    }
    let mut seen = std::collections::HashSet::new();
    urls.retain(|url| seen.insert(url.clone()));
    urls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_config_removes_nonadjacent_duplicates_across_all_aliases_in_first_seen_order() {
        assert_eq!(
            BACKUP_KEYS,
            ["BACKUP_RPC_URL", "BACKUP_RPC_URLS", "RPC_BACKUP_URLS"]
        );
        let result = merge_urls("https://primary.example.invalid".into(), [
            Some("https://backup-one.example.invalid".into()),
            Some("https://backup-two.example.invalid,https://primary.example.invalid,https://backup-one.example.invalid".into()),
            Some("https://backup-three.example.invalid,https://backup-two.example.invalid".into()),
        ]);
        assert_eq!(
            result,
            vec![
                "https://primary.example.invalid",
                "https://backup-one.example.invalid",
                "https://backup-two.example.invalid",
                "https://backup-three.example.invalid"
            ]
        );
    }

    #[test]
    fn rpc_config_keeps_distinct_keys_on_the_same_provider_as_distinct_endpoints() {
        let one = "https://mainnet.helius-rpc.com/?api-key=fixture-one";
        let two = "https://mainnet.helius-rpc.com/?api-key=fixture-two";
        assert_eq!(
            merge_urls(one.into(), [Some(two.into()), None, None]),
            vec![one, two]
        );
    }

    #[test]
    fn rpc_config_preserves_primary_literal_and_only_trims_backup_entries() {
        let primary = " https://primary.example.invalid/ ";
        assert_eq!(
            merge_urls(
                primary.into(),
                [
                    Some(" , \t https://backup.example.invalid/ \n,, ".into()),
                    None,
                    None
                ]
            ),
            vec![primary, "https://backup.example.invalid/"]
        );
    }

    #[test]
    fn rpc_config_empty_backups_do_not_add_endpoints_or_change_primary() {
        assert_eq!(
            merge_urls(
                "https://primary.example.invalid".into(),
                [None, Some(" , \t,, ".into()), Some(String::new())]
            ),
            vec!["https://primary.example.invalid"]
        );
    }
}
