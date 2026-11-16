// Views for developer-service cards (stream D).
import SwiftUI
import SurfaceKit

struct ServiceCardView: View {
    let card: Card
    let service: ServiceCard
    let send: (Action) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            CardHeading(from: service.service, title: service.title)
            ForEach(service.rows.prefix(4), id: \.id) { row in
                Text(row.title).font(.caption).lineLimit(1)
            }
        }
    }
}
