// Views for Mail cards (stream E).
import SwiftUI
import SurfaceKit

struct MailCardView: View {
    let card: Card
    let mail: MailCard
    let send: (Action) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(mail.messages.prefix(4), id: \.id) { message in
                HStack(spacing: 6) {
                    Text(message.from).font(.caption.weight(message.unread ? .semibold : .regular)).lineLimit(1)
                    Text(message.subject).font(.caption).opacity(0.75).lineLimit(1)
                }
            }
        }
    }
}
