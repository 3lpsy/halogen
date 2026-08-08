package org.fgsec.halogen

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Test
import org.junit.runner.RunWith

/// Not a journey: signs into the recipe's server and exits, PERSISTING the
/// session — a follow-up plain launch resumes signed-in. Lets a human drive
/// an already-connected app over droiddriver without typing URLs through the
/// lossy remote-input path.
@RunWith(AndroidJUnit4::class)
class InteractiveSetupTest : JourneyCase() {

    @Test
    fun signInAndPersist() {
        launch(autoconnect = "$serverBase|dev|dev")
        requireSessionUp()
    }
}
