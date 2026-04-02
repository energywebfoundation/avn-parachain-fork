// AvN is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with AvN.  If not, see <http://www.gnu.org/licenses/>.

use crate::*;

use codec::{Decode, Encode};
use frame_support::{
    pallet_prelude::*,
    storage::storage_prefix,
    traits::{GetStorageVersion, OnRuntimeUpgrade, StorageInstance},
    weights::Weight,
    StorageHasher, Blake2_128Concat,
};
use frame_system::pallet_prelude::BlockNumberFor;
use sp_avn_common::bounds::{MaximumValidatorsBound, VotingSessionIdBound};
use sp_core::ecdsa;
use sp_std::vec::Vec;

pub const TARGET_STORAGE_VERSION: StorageVersion = StorageVersion::new(2);

/// Old `VotingSessionData` layout that includes the removed `confirmations` field.
/// Used to decode pre-t1t2comms entries so they can be re-encoded in the new format.
#[derive(Decode)]
pub struct OldVotingSessionData<AccountId, BlockNumber> {
    pub voting_session_id: BoundedVec<u8, VotingSessionIdBound>,
    pub threshold: u32,
    pub ayes: BoundedVec<AccountId, MaximumValidatorsBound>,
    pub nays: BoundedVec<AccountId, MaximumValidatorsBound>,
    pub end_of_voting_period: BlockNumber,
    pub confirmations: BoundedVec<ecdsa::Signature, MaximumValidatorsBound>,
    pub created_at_block: BlockNumber,
}

/// Migration to re-encode `VotesRepository` entries, dropping the removed `confirmations` field
pub struct MigrateVotesRepository<T, I = ()>(PhantomData<(T, I)>);
impl<T: Config<I>, I: 'static> OnRuntimeUpgrade for MigrateVotesRepository<T, I> {
    fn on_runtime_upgrade() -> Weight {
        let onchain = Pallet::<T, I>::on_chain_storage_version();

        if onchain < 2 {
            log::info!(
                "💽 Summary VotesRepository migration: re-encoding stale entries (onchain: {:?} -> target: {:?})",
                onchain,
                TARGET_STORAGE_VERSION
            );

            let mut already_valid: u64 = 0;
            let mut migrated: u64 = 0;
            let mut removed: u64 = 0;

            let prefix = storage_prefix(
                <Pallet<T, I> as PalletInfoAccess>::name().as_bytes(),
                b"VotesRepository",
            );

            let mut previous_key = prefix.to_vec();
            while let Some(next_key) =
                sp_io::storage::next_key(&previous_key)
            {
                // Stop if we've left the VotesRepository prefix
                if !next_key.starts_with(&prefix) {
                    break;
                }

                let raw_value = match sp_io::storage::get(&next_key) {
                    Some(v) => v,
                    None => {
                        previous_key = next_key;
                        continue;
                    },
                };

                // Try decoding as the current type, ensuring all bytes are consumed
                let mut cursor = &raw_value[..];
                if VotingSessionData::<T::AccountId, BlockNumberFor<T>>::decode(
                    &mut cursor,
                ).is_ok() && cursor.is_empty() {
                    // Already in the new format
                    already_valid += 1;
                    previous_key = next_key;
                    continue;
                }

                // Try decoding as the old type (with confirmations)
                let mut cursor = &raw_value[..];
                match OldVotingSessionData::<T::AccountId, BlockNumberFor<T>>::decode(
                    &mut cursor,
                ) {
                    Ok(old) => {
                        let new = VotingSessionData {
                            voting_session_id: old.voting_session_id,
                            threshold: old.threshold,
                            ayes: old.ayes,
                            nays: old.nays,
                            end_of_voting_period: old.end_of_voting_period,
                            created_at_block: old.created_at_block,
                        };
                        sp_io::storage::set(&next_key, &new.encode());
                        migrated += 1;
                    },
                    Err(_) => {
                        // Neither format decodes, remove corrupted entry
                        log::warn!(
                            "⚠️ Summary VotesRepository: removing undecodable entry at key {:?}",
                            sp_core::hexdisplay::HexDisplay::from(&next_key)
                        );
                        sp_io::storage::clear(&next_key);
                        removed += 1;
                    },
                }

                previous_key = next_key;
            }

            log::info!(
                "✅ Summary VotesRepository migration completed: {} already valid, {} migrated, {} removed",
                already_valid,
                migrated,
                removed
            );

            TARGET_STORAGE_VERSION.put::<Pallet<T, I>>();

            let total = already_valid + migrated + removed;
            return T::DbWeight::get().reads_writes(total + 1, migrated + removed + 1)
        }

        log::info!(
            "💽 Summary VotesRepository migration: skipped (already at version {:?})",
            onchain
        );
        Weight::zero()
    }

    #[cfg(feature = "try-runtime")]
    fn pre_upgrade() -> Result<Vec<u8>, sp_runtime::TryRuntimeError> {
        let prefix = storage_prefix(
            <Pallet<T, I> as PalletInfoAccess>::name().as_bytes(),
            b"VotesRepository",
        );

        let mut count: u64 = 0;
        let mut previous_key = prefix.to_vec();
        while let Some(next_key) = sp_io::storage::next_key(&previous_key) {
            if !next_key.starts_with(&prefix) {
                break;
            }
            count += 1;
            previous_key = next_key;
        }

        log::info!(
            "💽 Summary VotesRepository migration: pre_upgrade - {} raw keys found",
            count
        );
        Ok(count.encode())
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(state: Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
        let onchain = Pallet::<T, I>::on_chain_storage_version();
        frame_support::ensure!(onchain == 2, "Summary storage version must be 2 after migration");

        let count_before = u64::decode(&mut &state[..])
            .map_err(|_| "Failed to decode pre_upgrade state")?;
        let count_after = VotesRepository::<T, I>::iter().count() as u64;

        log::info!(
            "✅ Summary VotesRepository migration: post_upgrade - {} entries before, {} decodable entries after",
            count_before,
            count_after
        );

        // Verify all surviving entries are decodable
        frame_support::ensure!(
            count_after <= count_before,
            "Entry count increased unexpectedly after migration"
        );

        Ok(())
    }
}
