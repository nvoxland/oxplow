---
name: setup-java-reports
description: Make a Java project's Maven or Gradle test run write the JUnit and JaCoCo reports oxplow reads. Use when setting up collection for a Java project, or when its test runs show no individual tests or no coverage.
---

# Java reports for oxplow

oxplow reads JUnit XML (the individual tests) and a JaCoCo XML report
(coverage).

1. JUnit: Maven Surefire and Gradle already write JUnit XML.
   - Maven: `target/surefire-reports/TEST-*.xml`.
   - Gradle: `build/test-results/test/TEST-*.xml`.
2. Coverage: add the JaCoCo plugin and turn on its XML report.
   - Maven: add `jacoco-maven-plugin` with the `prepare-agent` and
     `report` goals. The report is `target/site/jacoco/jacoco.xml`.
   - Gradle: apply the `jacoco` plugin. Set
     `jacocoTestReport { reports { xml.required = true } }` and make
     `test` finalized by `jacocoTestReport`. The report is
     `build/reports/jacoco/test/jacocoTestReport.xml`.
3. Declare each file as a report collector in the project's extension
   (`.oxplow/extensions/project/extension.yaml`). The test parser is
   `oxplow:junit`. The coverage parser is `oxplow:jacoco`.
4. Make the smallest change that works. Leave the diff for the person
   to review.
