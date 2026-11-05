These public Apple-issued certificate fixtures are copied unchanged from
`apple-codesign` 0.29.0, `src/testdata/apple-signed-apple-development.cer` and
`src/testdata/apple-signed-developer-id-application.cer` respectively. The
upstream source is <https://crates.io/crates/apple-codesign/0.29.0>.

The fixture files are distributed under the Mozilla Public License 2.0, the
license of `apple-codesign`. The full license text is available at
<https://www.mozilla.org/MPL/2.0/>.

`fastlane-test.mobileprovision` is copied unchanged from fastlane's public
`fastlane/spec/fixtures/profiles/test.mobileprovision` fixture at
<https://github.com/fastlane/fastlane/blob/master/fastlane/spec/fixtures/profiles/test.mobileprovision>.
It is an Apple-signed iOS provisioning profile used only to test CMS signer
identity and chain validation; its profile metadata may be expired. Fastlane's
MIT license is included as `LICENSE-fastlane-MIT.txt`.

The following unchanged, Apple-signed profiles come from dotnet/macios commit
`27b2ccb2f13349d3767f4cf69c904e46876bc81a`, directory
[`tests/msbuild/Xamarin.MacDev.Tasks.Tests/Resources`](https://github.com/dotnet/macios/tree/27b2ccb2f13349d3767f4cf69c904e46876bc81a/tests/msbuild/Xamarin.MacDev.Tasks.Tests/Resources).
The upstream MIT license is included as `LICENSE-dotnet-MIT.txt`.

| Local file | Upstream file | SHA-256 |
| --- | --- | --- |
| `der-ios.mobileprovision` | `APS_Development_Environment_Profile.mobileprovision` | `db6376c04478b62cd2d395f608f6dcc7460e9025d91d3f2e2c10d4cb642d6493` |
| `der-ios-signin.mobileprovision` | `Apple_Signin_Test_Profile.mobileprovision` | `a4d867cac0d644f7a1d40781d39f736695778c1a369d92fd812e700cf3d42842` |
| `der-macos.provisionprofile` | `WildCardMacAppDevelopment.provisionprofile` | `de4810ae19d154da90a0a8828bbcbde3dbd33d6fab6bc2de0e1910536241caa6` |

These expired profiles exercise both CMS signatures and chains, DER/XML
consistency, date conversion, SHA-256 certificate fingerprints, iOS push and
Sign in with Apple entitlements, and macOS wildcard/array entitlements. Tests
isolate consistency from profile expiry; production still rejects expiry.
The representation follows Apple's [TN3125](https://developer.apple.com/documentation/technotes/tn3125-inside-code-signing-provisioning-profiles#The-future-is-DER).
No private signing keys are included.
