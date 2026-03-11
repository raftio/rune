/// Pure network-isolation logic for rune-network.
///
/// Two agents may communicate only when their network lists share at least one
/// common name.  Both sides fall back to `["bridge"]` when their list is empty.
pub struct NetworkPolicy;

impl NetworkPolicy {
    /// Returns `true` when `caller_networks` and `callee_networks` intersect.
    ///
    /// Empty slices are treated as `["bridge"]` (the implicit default network).
    pub fn check_access(caller_networks: &[String], callee_networks: &[String]) -> bool {
        let bridge = vec!["bridge".to_string()];
        let caller = if caller_networks.is_empty() { &bridge } else { caller_networks };
        let callee = if callee_networks.is_empty() { &bridge } else { callee_networks };
        caller.iter().any(|n| callee.contains(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nets(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn same_network_allowed() {
        assert!(NetworkPolicy::check_access(&nets(&["prod"]), &nets(&["prod"])));
    }

    #[test]
    fn disjoint_networks_denied() {
        assert!(!NetworkPolicy::check_access(&nets(&["prod"]), &nets(&["staging"])));
    }

    #[test]
    fn overlapping_subset_allowed() {
        assert!(NetworkPolicy::check_access(
            &nets(&["prod", "shared"]),
            &nets(&["staging", "shared"]),
        ));
    }

    #[test]
    fn empty_defaults_to_bridge() {
        // Both empty → both bridge → allowed.
        assert!(NetworkPolicy::check_access(&[], &[]));
    }

    #[test]
    fn one_empty_defaults_to_bridge() {
        // Empty caller ↔ explicit bridge → allowed.
        assert!(NetworkPolicy::check_access(&[], &nets(&["bridge"])));
        // Empty caller ↔ prod only → denied.
        assert!(!NetworkPolicy::check_access(&[], &nets(&["prod"])));
    }
}
