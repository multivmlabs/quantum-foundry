use alloy_primitives::Bytes;
use alloy_sol_types::{SolCall, sol};

sol! {
    #[sol(abi)]
    interface KeyVaultLifecycle {
        function bootstrapKey();
        function addKey(
            uint32 keyId,
            bytes pubkey,
            uint8 scheme,
            bytes authProof,
            uint8 cosignerScheme,
            uint8 scopedPermissions,
            bytes scopeData,
        );
        function removeKey(uint32 keyId);
        function updateKeyAuth(
            uint32 keyId,
            bytes authProof,
            uint8 scheme,
            bytes scopeData,
            uint8 scopedPermissions,
        );
    }
}

/// Inputs for a KeyVault `addKey` lifecycle write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuantumAddKeyInputs {
    pub target_key_id: u32,
    pub pubkey: Bytes,
    pub scheme: u8,
    pub auth_proof: Bytes,
    pub cosigner_scheme: u8,
    pub scoped_permissions: u8,
    pub scope_data: Bytes,
}

/// Inputs for a KeyVault `updateKeyAuth` lifecycle write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuantumUpdateKeyAuthInputs {
    pub target_key_id: u32,
    pub auth_proof: Bytes,
    pub scheme: u8,
    pub scope_data: Bytes,
    pub scoped_permissions: u8,
}

/// Build calldata for `bootstrapKey()`.
pub fn encode_bootstrap_calldata() -> Bytes {
    KeyVaultLifecycle::bootstrapKeyCall {}.abi_encode().into()
}

/// Build calldata for `addKey(...)`.
pub fn encode_add_key_calldata(inputs: &QuantumAddKeyInputs) -> Bytes {
    KeyVaultLifecycle::addKeyCall {
        keyId: inputs.target_key_id,
        pubkey: inputs.pubkey.to_vec().into(),
        scheme: inputs.scheme,
        authProof: inputs.auth_proof.to_vec().into(),
        cosignerScheme: inputs.cosigner_scheme,
        scopedPermissions: inputs.scoped_permissions,
        scopeData: inputs.scope_data.to_vec().into(),
    }
    .abi_encode()
    .into()
}

/// Build calldata for `removeKey(uint32)`.
pub fn encode_remove_key_calldata(target_key_id: u32) -> Bytes {
    KeyVaultLifecycle::removeKeyCall { keyId: target_key_id }.abi_encode().into()
}

/// Build calldata for `updateKeyAuth(...)`.
pub fn encode_update_key_auth_calldata(inputs: &QuantumUpdateKeyAuthInputs) -> Bytes {
    KeyVaultLifecycle::updateKeyAuthCall {
        keyId: inputs.target_key_id,
        authProof: inputs.auth_proof.to_vec().into(),
        scheme: inputs.scheme,
        scopeData: inputs.scope_data.to_vec().into(),
        scopedPermissions: inputs.scoped_permissions,
    }
    .abi_encode()
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        QUANTUM_ADD_KEY_SELECTOR, QUANTUM_BOOTSTRAP_SELECTOR, QUANTUM_REMOVE_KEY_SELECTOR,
        QUANTUM_UPDATE_KEY_AUTH_SELECTOR,
    };

    #[test]
    fn bootstrap_calldata_matches_frozen_selector() {
        let calldata = encode_bootstrap_calldata();
        assert_eq!(&calldata[..4], &QUANTUM_BOOTSTRAP_SELECTOR);
        assert_eq!(calldata.len(), 4);
    }

    #[test]
    fn remove_key_calldata_matches_frozen_selector() {
        let calldata = encode_remove_key_calldata(7);
        assert_eq!(&calldata[..4], &QUANTUM_REMOVE_KEY_SELECTOR);
        assert_eq!(calldata.len(), 4 + 32);
    }

    #[test]
    fn add_key_calldata_matches_frozen_selector() {
        let calldata = encode_add_key_calldata(&QuantumAddKeyInputs {
            target_key_id: 1,
            pubkey: Bytes::from_static(&[1, 2, 3]),
            scheme: 1,
            auth_proof: Bytes::from_static(&[4, 5]),
            cosigner_scheme: 0,
            scoped_permissions: 0,
            scope_data: Bytes::new(),
        });
        assert_eq!(&calldata[..4], &QUANTUM_ADD_KEY_SELECTOR);
    }

    #[test]
    fn update_key_auth_calldata_matches_frozen_selector() {
        let calldata = encode_update_key_auth_calldata(&QuantumUpdateKeyAuthInputs {
            target_key_id: 2,
            auth_proof: Bytes::from_static(&[9, 9]),
            scheme: 1,
            scope_data: Bytes::new(),
            scoped_permissions: 0,
        });
        assert_eq!(&calldata[..4], &QUANTUM_UPDATE_KEY_AUTH_SELECTOR);
    }

    #[test]
    fn round_trip_add_key_decodes_to_inputs() {
        let inputs = QuantumAddKeyInputs {
            target_key_id: 5,
            pubkey: Bytes::from_static(&[0xaa; 33]),
            scheme: 2,
            auth_proof: Bytes::from_static(&[0xbb; 64]),
            cosigner_scheme: 2,
            scoped_permissions: 1,
            scope_data: Bytes::from_static(&[0xcc; 8]),
        };
        let calldata = encode_add_key_calldata(&inputs);
        let decoded =
            KeyVaultLifecycle::addKeyCall::abi_decode_raw(&calldata[4..]).unwrap();
        assert_eq!(decoded.keyId, inputs.target_key_id);
        assert_eq!(decoded.pubkey.as_ref(), inputs.pubkey.as_ref());
        assert_eq!(decoded.scheme, inputs.scheme);
        assert_eq!(decoded.authProof.as_ref(), inputs.auth_proof.as_ref());
        assert_eq!(decoded.cosignerScheme, inputs.cosigner_scheme);
        assert_eq!(decoded.scopedPermissions, inputs.scoped_permissions);
        assert_eq!(decoded.scopeData.as_ref(), inputs.scope_data.as_ref());
    }
}
