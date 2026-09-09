# Current presentation system

`halogen.css` and `halogen.js` provide the shared HTML mock components. Native
iOS uses its current blue tint, black content background and system list/form
shapes. The `.web` scope preserves Dioxus's gray theme, cards, sidebar and dock.
A platform variant changes chrome and capabilities, not podcast data semantics.

`01-components.html` records the reusable shapes. These are approximations of
native controls; the Swift source and actual screenshots resolve differences.
Assets and their provenance live in `assets/`.

Each standalone sheet links both shared files. Keep frame labels outside the
mock, put source/capability notes in captions, and rebuild `../master.html`
after changing sheets. Current-state mocks do not grant approval for new UI.
