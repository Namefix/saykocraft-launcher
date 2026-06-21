use std::cmp::Ordering;

fn parse_semver(version: &str) -> Option<(u64, u64, u64, Option<&str>)> {
    let version = version.strip_prefix('v').unwrap_or(version);
    let (core, pre_release) = version
        .split_once('-')
        .map_or((version, None), |(core, pre_release)| {
            (core, Some(pre_release))
        });
    let mut parts = core.split('.');

    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;

    if parts.next().is_some() {
        return None;
    }

    Some((major, minor, patch, pre_release))
}

fn compare_identifiers(left: &str, right: &str) -> Ordering {
    match (left.parse::<u64>(), right.parse::<u64>()) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        (Err(_), Err(_)) => left.cmp(right),
    }
}

pub fn compare_semver(left: &str, right: &str) -> Option<Ordering> {
    let (left_major, left_minor, left_patch, left_pre) = parse_semver(left)?;
    let (right_major, right_minor, right_patch, right_pre) = parse_semver(right)?;

    Some(
        (left_major, left_minor, left_patch)
            .cmp(&(right_major, right_minor, right_patch))
            .then_with(|| match (left_pre, right_pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(left_pre), Some(right_pre)) => {
                    let mut left_parts = left_pre.split('.');
                    let mut right_parts = right_pre.split('.');

                    loop {
                        match (left_parts.next(), right_parts.next()) {
                            (None, None) => break Ordering::Equal,
                            (None, Some(_)) => break Ordering::Less,
                            (Some(_), None) => break Ordering::Greater,
                            (Some(left_part), Some(right_part)) => {
                                let ordering = compare_identifiers(left_part, right_part);
                                if ordering != Ordering::Equal {
                                    break ordering;
                                }
                            }
                        }
                    }
                }
            }),
    )
}
