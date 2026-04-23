// TODO (Phase: Android) — implement JNI bindings here.
//
// Steps:
// 1. Add `jni` dependency under [target.'cfg(target_os = "android")'.dependencies]
// 2. Implement GpgsClient using cargo-mobile2 JNI bridge
// 3. pull():  call PlayGames.getSnapshotsClient().open("solitaire_quest_sync")
//             -> deserialize JSON blob into SyncPayload
// 4. push():  serialize SyncPayload to JSON -> write to Saved Game slot
// 5. mirror_achievement(id): call PlayGames.getAchievementsClient().unlock(map_id(id))
// 6. Maintain a static ID mapping: our &str IDs -> GPGS achievement IDs (from Play Console)
// 7. On GameWonEvent, submit score to GPGS leaderboard
// 8. Add Google Sign-In button to Settings screen (Android build only, #[cfg] gated)
