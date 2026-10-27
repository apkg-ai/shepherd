-- Immutable-record events written before step 015 carried revision 0; the
-- stable contract floor is 1 (an immutable record's single revision).
UPDATE events SET resource_revision = 1
WHERE resource_revision = 0 AND type IN ('session.recorded', 'review.recorded');
