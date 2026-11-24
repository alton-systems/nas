//! Ponte entre `Effect` e a política de permissões de
//! `nas-history` (ADR 0006 §5, etapa 3d) — a leitura/validação do
//! arquivo em si é toda de `nas-history` (fica ao lado do
//! journal/checkpoints, fora do projeto); aqui só se decide se um
//! efeito específico está liberado por ela.

use nas_history::PermissionsPolicy;

use crate::effect::Effect;

/// `effect` está liberado, sem perguntar, pela política deste
/// projeto? Verdadeiro de cara pra qualquer efeito sem
/// [`Effect::policy_key`] (não pede aprovação nenhuma); pros três que
/// pedem, lê o booleano correspondente.
pub fn allowed_by_policy(effect: Effect, policy: &PermissionsPolicy) -> bool {
    match effect {
        Effect::WriteFile => policy.write_file,
        Effect::Shell => policy.shell,
        Effect::Network => policy.network,
        Effect::Pure | Effect::ReadFile | Effect::WriteBuffer | Effect::WriteHistory => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn efeitos_sem_aprovacao_sempre_liberados() {
        let policy = PermissionsPolicy::default();
        assert!(allowed_by_policy(Effect::Pure, &policy));
        assert!(allowed_by_policy(Effect::ReadFile, &policy));
        assert!(allowed_by_policy(Effect::WriteBuffer, &policy));
        assert!(allowed_by_policy(Effect::WriteHistory, &policy));
    }

    #[test]
    fn efeitos_com_aprovacao_seguem_a_politica() {
        let negado = PermissionsPolicy::default();
        assert!(!allowed_by_policy(Effect::WriteFile, &negado));
        assert!(!allowed_by_policy(Effect::Shell, &negado));
        assert!(!allowed_by_policy(Effect::Network, &negado));

        let liberado = PermissionsPolicy {
            write_file: true,
            shell: true,
            network: true,
        };
        assert!(allowed_by_policy(Effect::WriteFile, &liberado));
        assert!(allowed_by_policy(Effect::Shell, &liberado));
        assert!(allowed_by_policy(Effect::Network, &liberado));
    }
}
