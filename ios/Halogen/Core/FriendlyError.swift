import Foundation

/// Human copy for every failure class — never the raw enum/NSError dump
/// (web: the classify funnel + auth error_message mapping). Every inline
/// error slot, toast, and the sync-failure surface routes through here;
/// screens with context-specific copy (ConnectView) override then delegate.
enum FriendlyError {
    static func message(_ error: Error) -> String {
        switch error {
        case let client as HalogenClient.ClientError:
            switch client {
            case .api(let fields):
                let messages = fields.values.flatMap { $0 }.compactMap(\.message)
                return messages.isEmpty
                    ? "The server rejected the request."
                    : messages.joined(separator: " · ")
            case .http(401):
                return "Session expired — sign in again."
            case .http(404):
                return "Not found on the server — it may have been removed."
            case .http(let status) where status >= 500:
                return "The server hit an error (\(status)). Try again in a moment."
            case .http(let status):
                return "The server refused the request (HTTP \(status))."
            case .offline:
                return "You're offline — reconnect to do this."
            case .signedOut:
                return "Not signed in."
            case .emptyData:
                return "The server answered with an unexpected response."
            }
        case let url as URLError:
            switch url.code {
            case .cannotFindHost, .dnsLookupFailed:
                return "Can't find the server — check the address."
            case .cannotConnectToHost, .networkConnectionLost, .timedOut:
                return "Can't reach the server right now — try again."
            case .notConnectedToInternet, .dataNotAllowed:
                return "You're offline — connect to a network first."
            case .secureConnectionFailed, .serverCertificateUntrusted,
                .serverCertificateHasBadDate, .serverCertificateHasUnknownRoot,
                .serverCertificateNotYetValid:
                return "Secure connection failed — check the server's certificate."
            case .cancelled:
                return "Cancelled."
            default:
                return "Network request failed — check the connection and try again."
            }
        case is DecodingError:
            return "Couldn't read the server's response — app and server versions may not match."
        case let embedded as HalogenCore.EmbeddedUserError:
            return embedded.description
        case let localized as LocalizedError:
            return localized.errorDescription ?? "Something went wrong — try again."
        default:
            return "Something went wrong — try again."
        }
    }
}
