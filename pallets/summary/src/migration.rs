// AvN is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with AvN.  If not, see <http://www.gnu.org/licenses/>.

pub mod v2 {
    use crate::*;

    use codec::{Decode, Encode};
    use frame_support::{
        pallet_prelude::*,
        storage::storage_prefix,
        traits::OnRuntimeUpgrade,
        weights::Weight,
    };
    use frame_system::pallet_prelude::BlockNumberFor;
    use sp_avn_common::bounds::{MaximumValidatorsBound, VotingSessionIdBound};
    use sp_core::ecdsa;

    const LOG_TARGET: &'static str = "runtime::summary::migration::v2";

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

    pub struct VersionUncheckedMigrateV1ToV2<T, I = ()>(PhantomData<(T, I)>);
    impl<T: Config<I>, I: 'static> OnRuntimeUpgrade
        for VersionUncheckedMigrateV1ToV2<T, I>
    {
        fn on_runtime_upgrade() -> Weight {
            log::info!(target: LOG_TARGET, "migrating VotesRepository entries");

            let mut already_valid: u64 = 0;
            let mut migrated: u64 = 0;
            let mut removed: u64 = 0;

            let prefix = storage_prefix(
                <Pallet<T, I> as PalletInfoAccess>::name().as_bytes(),
                b"VotesRepository",
            );

            // Raw key iteration: storage may contain a mix of old-format and already-migrated
            // entries, so `translate()` cannot be used safely.
            let mut previous_key = prefix.to_vec();
            while let Some(next_key) = sp_io::storage::next_key(&previous_key) {
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

                let mut cursor = &raw_value[..];
                if VotingSessionData::<T::AccountId, BlockNumberFor<T>>::decode(&mut cursor)
                    .is_ok()
                    && cursor.is_empty()
                {
                    already_valid = already_valid.saturating_add(1);
                    previous_key = next_key;
                    continue;
                }

                let mut cursor = &raw_value[..];
                match OldVotingSessionData::<T::AccountId, BlockNumberFor<T>>::decode(&mut cursor)
                {
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
                        migrated = migrated.saturating_add(1);
                    },
                    Err(_) => {
                        log::warn!(
                            target: LOG_TARGET,
                            "removing undecodable entry at key {:?}",
                            sp_core::hexdisplay::HexDisplay::from(&next_key)
                        );
                        sp_io::storage::clear(&next_key);
                        removed = removed.saturating_add(1);
                    },
                }

                previous_key = next_key;
            }

            log::info!(
                target: LOG_TARGET,
                "completed: {} already valid, {} migrated, {} removed",
                already_valid, migrated, removed
            );

            let total = already_valid
                .saturating_add(migrated)
                .saturating_add(removed);

            T::DbWeight::get()
                .reads(total)
                .saturating_add(T::DbWeight::get().writes(migrated.saturating_add(removed)))
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
                count = count.saturating_add(1);
                previous_key = next_key;
            }

            log::info!(target: LOG_TARGET, "pre_upgrade: {} raw keys found", count);
            Ok(count.encode())
        }

        #[cfg(feature = "try-runtime")]
        fn post_upgrade(state: Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
            let count_before = u64::decode(&mut &state[..])
                .map_err(|_| "Failed to decode pre_upgrade state")?;

            let prefix = storage_prefix(
                <Pallet<T, I> as PalletInfoAccess>::name().as_bytes(),
                b"VotesRepository",
            );

            let mut count_after: u64 = 0;
            let mut previous_key = prefix.to_vec();
            while let Some(next_key) = sp_io::storage::next_key(&previous_key) {
                if !next_key.starts_with(&prefix) {
                    break;
                }
                count_after = count_after.saturating_add(1);
                previous_key = next_key;
            }

            log::info!(
                target: LOG_TARGET,
                "post_upgrade: {} entries before, {} entries after",
                count_before, count_after
            );

            frame_support::ensure!(
                count_after <= count_before,
                "Entry count increased unexpectedly after migration"
            );

            for (_key, _value) in VotesRepository::<T, I>::iter() {}

            Ok(())
        }
    }

    pub type MigrateV1ToV2<T, I = ()> = frame_support::migrations::VersionedMigration<
        1,
        2,
        VersionUncheckedMigrateV1ToV2<T, I>,
        Pallet<T, I>,
        <T as frame_system::Config>::DbWeight,
    >;
}
